use crate::process::{protocol, Endpoint, Invalidation};
use port_platform::{
    accessibility::{Accessibility, ExtractionTarget},
    authority::{ReadSequence, TargetAuthority, TargetReadKey},
    capture::{
        Capture, CaptureBatch, CaptureCoverage, CaptureOmission, CaptureOmissionReason,
        CaptureRegion, CaptureRequest, CaptureTarget, CapturedImage,
    },
    geometry::{AcquiredFrame, DesktopMapping, DesktopSpace, PixelExtent, PixelRect},
    model::{
        Bounds, ExtractedNode, ExtractionMetrics, ExtractionQuality, ExtractionResult, NodePurpose,
        ResolvedWindow, SelectedWindow, SemanticRole, SourceApi, WindowIdentity,
        WindowObservableFacts,
    },
    observation::{
        Observation, ObservationEvents, ObservationRegistration, ObservationRequest,
        ObservationSession, WindowObservationEvent, WindowObservationNotification,
        WindowObservationStart,
    },
    selection::{RegisteredWindow, TargetSelection, WindowPickerReply},
    trust::AccessibilityTrust,
    ExtractionLimits, ImageCaptureLimits, PlatformError, PlatformFuture,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex, MutexGuard,
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use uuid::Uuid;

const STEP: Duration = Duration::from_secs(6);
const SELECTION: Duration = Duration::from_secs(60);
const PNG_LIMIT: usize = 8 * 1024 * 1024;
const CONTINUOUS_READ_LIMIT: u64 = 3;

#[derive(Clone, Copy, Debug)]
pub enum Fixture {
    A,
    B,
}

impl Fixture {
    fn name(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    fn title(self) -> String {
        format!("Lens fixture {}", self.name())
    }

    fn ordinal(self) -> u32 {
        match self {
            Self::A => 1,
            Self::B => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub enum FixtureReadProbe {
    #[default]
    Normal,
    Stall,
    Late,
}

struct ReadProbe {
    sequence: ReadSequence,
    mode: FixtureReadProbe,
    waiting: AtomicBool,
    // Written by the development host only after terminal admission.
    disposition: AtomicU8,
}

impl Default for ReadProbe {
    fn default() -> Self {
        Self {
            sequence: ReadSequence::FIRST,
            mode: FixtureReadProbe::Normal,
            waiting: AtomicBool::new(false),
            disposition: AtomicU8::new(0),
        }
    }
}

impl ReadProbe {
    const RESUME: u8 = 1;
    const INTERRUPT: u8 = 2;

    fn mode_for(&self, sequence: ReadSequence) -> FixtureReadProbe {
        if self.sequence == sequence {
            self.mode
        } else {
            FixtureReadProbe::Normal
        }
    }

    fn read(
        &self,
        worker: &mut Endpoint,
        channel: &str,
        sequence: ReadSequence,
        deadline: Instant,
    ) -> Result<Value, PlatformError> {
        let number = sequence.get().to_string();
        let mode = self.mode_for(sequence);
        let suffix = match mode {
            FixtureReadProbe::Normal => "",
            FixtureReadProbe::Stall => " stall",
            FixtureReadProbe::Late => " late",
        };
        worker.send(&format!("read {number}{suffix}"))?;
        if !matches!(mode, FixtureReadProbe::Normal) {
            let waiting = worker.expect("validation-wait", deadline)?;
            check_channel(&waiting, channel)?;
            if waiting["read"] != number {
                return Err(protocol("unexpected validation read"));
            }
            self.waiting.store(true, Ordering::Release);
            while self.disposition.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            self.waiting.store(false, Ordering::Release);
            match self.disposition.load(Ordering::Acquire) {
                Self::RESUME if matches!(mode, FixtureReadProbe::Late) => worker.send("resume")?,
                Self::INTERRUPT => {
                    return Err(protocol("validation read interrupted before native result"))
                }
                _ => return Err(protocol("validation wait deadline elapsed")),
            }
        }
        worker.expect("result", deadline)
    }
}

/// The caller explicitly supplies the directory built from this package's fixtures.
/// Selection accepts a fixture variant only. It cannot discover or bind external windows.
pub struct FixturePlatform {
    binaries: PathBuf,
    selection: Fixture,
    slot: Mutex<Option<Registration>>,
    read_limit: u64,
    probe: ReadProbe,
}

impl FixturePlatform {
    pub fn new(binaries: &Path, selection: Fixture) -> Result<Self, PlatformError> {
        let binaries = binaries.canonicalize().map_err(protocol)?;
        if ["fixture", "worker", "selector"]
            .iter()
            .any(|name| !binary(&binaries, name).is_file())
        {
            return Err(protocol("building the three fixture binaries is required"));
        }
        Ok(Self {
            binaries,
            selection,
            slot: Mutex::new(None),
            read_limit: 1,
            probe: ReadProbe::default(),
        })
    }

    /// Opt into a bounded fault fixture. Normal acquisition never uses this gate.
    pub fn with_read_probe(self, mode: FixtureReadProbe) -> Self {
        self.with_read_probe_at(ReadSequence::FIRST, mode)
    }

    pub fn with_read_probe_at(mut self, sequence: ReadSequence, mode: FixtureReadProbe) -> Self {
        self.probe.sequence = sequence;
        self.probe.mode = mode;
        self
    }

    /// Retain the cooperative registration for up to three sequential, paired reads.
    pub fn with_continuous_reads(mut self) -> Self {
        self.read_limit = CONTINUOUS_READ_LIMIT;
        self
    }

    pub fn validation_waiting(&self) -> bool {
        self.probe.waiting.load(Ordering::Acquire)
    }

    /// The host must close its admission ledger before releasing or interrupting the probe.
    pub fn interrupt_validation_read(&self) {
        self.probe
            .disposition
            .store(ReadProbe::INTERRUPT, Ordering::Release);
    }

    /// Allow a real worker result to arrive after Stop for a negative publication test.
    pub fn deliver_validation_result(&self) {
        self.probe
            .disposition
            .store(ReadProbe::RESUME, Ordering::Release);
    }

    fn slot(&self) -> Result<MutexGuard<'_, Option<Registration>>, PlatformError> {
        self.slot
            .lock()
            .map_err(|_| protocol("fixture registry poisoned"))
    }

    /// Final native check before the lifecycle owner accepts the built common read.
    /// The host must serialize this with its own admission state and publication.
    pub fn approve_first_publication(&self, key: TargetReadKey) -> Result<(), PlatformError> {
        if key.sequence != ReadSequence::FIRST {
            return Err(protocol("first publication requires the first read"));
        }
        self.approve_read_publication(key)
    }

    /// The host must still admit this exact read under its publication lock.
    pub fn approve_read_publication(&self, key: TargetReadKey) -> Result<(), PlatformError> {
        let mut slot = self.slot()?;
        let registration = current(&mut slot, key)?;
        let sample = registration.sample(key)?;
        if sample.committed || !sample.transferred {
            return Err(protocol("publication requires one uncommitted paired read"));
        }
        registration.verify_owner("commit", Instant::now() + STEP)?;
        registration.sample(key)?.committed = true;
        Ok(())
    }

    /// Validation evidence only, not a shared capture capability. Return the
    /// bounded original PNG for the current exact paired read, before release.
    pub fn validation_frame_png(&self, key: TargetReadKey) -> Result<Vec<u8>, PlatformError> {
        let mut slot = self.slot()?;
        let registration = current(&mut slot, key)?;
        registration.sample(key)?;
        let mut png = Vec::new();
        File::open(
            registration
                .scratch
                .path()
                .join(format!("read-{}.png", key.sequence.get())),
        )
        .map_err(protocol)?
        .take(PNG_LIMIT as u64 + 1)
        .read_to_end(&mut png)
        .map_err(protocol)?;
        if png.len() > PNG_LIMIT {
            return Err(protocol(
                "validation frame exceeded the existing PNG budget",
            ));
        }
        registration.invalid.check()?;
        Ok(png)
    }

    /// Destroy the window in the cooperative owner without replacing its native identity.
    pub fn destroy_owned_fixture(&self, target: TargetAuthority) -> Result<(), PlatformError> {
        let mut slot = self.slot()?;
        let registration = slot
            .as_mut()
            .filter(|r| r.target == target)
            .ok_or_else(|| protocol("fixture registration unavailable"))?;
        registration.verify_owner("validate", Instant::now() + STEP)?;
        let reply = owner_request(
            &mut registration.owner,
            "destroy",
            &registration.reservation.nonce,
            Instant::now() + STEP,
        )?;
        check_reservation(&reply, &registration.reservation)
    }

    /// Controlled failure injection by the validation host. The registry can only
    /// address the worker it launched for this registration.
    pub fn exit_owned_worker(&self, target: TargetAuthority) -> Result<u32, PlatformError> {
        let mut slot = self.slot()?;
        let registration = slot
            .as_mut()
            .filter(|r| r.target == target)
            .ok_or_else(|| protocol("fixture registration unavailable"))?;
        registration.invalid.check()?;
        let pid = registration.worker.id();
        registration.worker.send("stop")?;
        registration.worker.close()?;
        Ok(pid)
    }

    /// Controlled failure injection by the validation host. The registry can only
    /// terminate the owner process it launched itself.
    pub fn terminate_owned_fixture(&self, target: TargetAuthority) -> Result<(), PlatformError> {
        let mut slot = self.slot()?;
        let registration = slot
            .as_mut()
            .filter(|r| r.target == target)
            .ok_or_else(|| protocol("fixture registration unavailable"))?;
        registration.owner.terminate()?;
        Ok(())
    }

    fn select(&self, operation_id: Uuid) -> Result<WindowPickerReply, PlatformError> {
        if operation_id.is_nil() {
            return Err(protocol("nil fixture operation"));
        }
        let mut slot = self.slot()?;
        if slot.is_some() {
            return Err(PlatformError::PickerBusy);
        }
        let Some(registration) =
            Registration::select(&self.binaries, self.selection, operation_id)?
        else {
            return Ok(WindowPickerReply::Cancelled);
        };
        let selected = RegisteredWindow {
            target: registration.target,
            window: registration.selected.clone(),
        };
        *slot = Some(registration);
        Ok(WindowPickerReply::Selected {
            windows: vec![selected],
        })
    }

    fn release(&self, matches: impl FnOnce(TargetAuthority) -> bool) -> Result<(), PlatformError> {
        let mut slot = self.slot()?;
        if slot.as_ref().is_some_and(|r| matches(r.target)) {
            // A failed teardown keeps the slot closed to new selections.
            slot.as_mut()
                .expect("matching registered fixture")
                .close()?;
            *slot = None;
        }
        Ok(())
    }
}

fn binary(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.exe"))
}

fn current(
    slot: &mut Option<Registration>,
    key: TargetReadKey,
) -> Result<&mut Registration, PlatformError> {
    let registration = slot
        .as_mut()
        .filter(|r| r.target == key.target)
        .ok_or_else(|| protocol("stale or unknown fixture registration"))?;
    registration.invalid.check()?;
    Ok(registration)
}

impl TargetSelection for FixturePlatform {
    fn pick(
        &self,
        operation_id: Uuid,
    ) -> PlatformFuture<'_, Result<WindowPickerReply, PlatformError>> {
        Box::pin(async move { self.select(operation_id) })
    }

    fn release_target(&self, target: TargetAuthority) -> Result<(), PlatformError> {
        self.release(|registered| registered == target)
    }

    fn release_operation(&self, operation_id: Uuid) -> Result<(), PlatformError> {
        self.release(|registered| registered.operation_id == operation_id)
    }
}

struct Registration {
    worker: Endpoint,
    owner: Endpoint,
    scratch: TempDir,
    invalid: Arc<Invalidation>,
    observing: Arc<AtomicBool>,
    selected: SelectedWindow,
    target: TargetAuthority,
    channel: String,
    reservation: Reservation,
    next_read: u64,
    sample: Option<Sample>,
}

#[derive(Deserialize)]
struct Reservation {
    nonce: String,
    granted: bool,
    generation: u64,
    handle: String,
    pid: u32,
    remaining_ms: u64,
}

struct Sample {
    frame: AcquiredFrame,
    png: Option<(Vec<u8>, image::DynamicImage)>,
    transferred: bool,
    committed: bool,
}

#[derive(Deserialize)]
struct NativeNode {
    name: Option<String>,
    role: i32,
    role_hr: i32,
    bounds_hr: i32,
    bounds: [i32; 4],
}

impl NativeNode {
    fn bounds(&self) -> Option<Bounds> {
        (self.bounds_hr == 0).then(|| bounds(self.bounds)).flatten()
    }
}

#[derive(Deserialize)]
struct NativeTree {
    root: NativeNode,
    children: Vec<Option<NativeNode>>,
    truncated: bool,
}

#[derive(Deserialize)]
struct NativeRead {
    channel: String,
    read: ReadSequence,
    width: u32,
    height: u32,
    dwm_before_hr: i32,
    dwm_after_hr: i32,
    dwm_before: [i32; 4],
    dwm_after: [i32; 4],
    uia: NativeTree,
}

fn bounds([x, y, right, bottom]: [i32; 4]) -> Option<Bounds> {
    (right > x && bottom > y).then_some(Bounds {
        x: f64::from(x),
        y: f64::from(y),
        width: f64::from(right) - f64::from(x),
        height: f64::from(bottom) - f64::from(y),
    })
}

fn owner_request(
    owner: &mut Endpoint,
    verb: &str,
    nonce: &str,
    deadline: Instant,
) -> Result<Reservation, PlatformError> {
    owner.send(&format!("{verb} {nonce}"))?;
    let response: Reservation =
        serde_json::from_value(owner.expect(verb, deadline)?).map_err(protocol)?;
    if response.nonce != nonce
        || response.pid != owner.id()
        || !response.granted
        || response.remaining_ms == 0
    {
        return Err(protocol(
            "fixture reservation missing, mismatched or expired",
        ));
    }
    Ok(response)
}

fn check_reservation(actual: &Reservation, expected: &Reservation) -> Result<(), PlatformError> {
    if actual.generation != expected.generation || actual.handle != expected.handle {
        return Err(protocol("fixture reservation changed window generation"));
    }
    Ok(())
}

impl Registration {
    fn select(
        binaries: &Path,
        fixture: Fixture,
        operation_id: Uuid,
    ) -> Result<Option<Self>, PlatformError> {
        let deadline = Instant::now() + SELECTION;
        let scratch = tempfile::tempdir().map_err(protocol)?;
        let invalid = Arc::new(Invalidation::default());
        let mut owner = Endpoint::spawn(
            &binary(binaries, "fixture"),
            scratch.path(),
            &[],
            Some(invalid.clone()),
        )?;
        owner.expect("ready", deadline)?;
        owner.send(match fixture {
            Fixture::A => "create-a",
            Fixture::B => "create-b",
        })?;
        let candidate = owner.expect("created", deadline)?;
        let nonce = Uuid::new_v4().simple().to_string();
        let reservation = owner_request(&mut owner, "reserve", &nonce, deadline)?;
        if candidate["marker"] != fixture.name()
            || candidate["pid"] != owner.id()
            || candidate["handle"] != reservation.handle
            || reservation.generation != 1
            || !reservation
                .handle
                .strip_prefix("0x")
                .is_some_and(|value| usize::from_str_radix(value, 16).is_ok_and(|hwnd| hwnd != 0))
        {
            return Err(protocol("owner did not reserve the explicit fixture"));
        }
        let channel = Uuid::new_v4().simple().to_string();
        let pid = owner.id().to_string();
        let mut worker = Endpoint::spawn(
            &binary(binaries, "worker"),
            scratch.path(),
            &[&reservation.handle, &pid, &channel],
            Some(invalid.clone()),
        )?;
        check_channel(&worker.expect("watch-ready", deadline)?, &channel)?;
        check_reservation(
            &owner_request(&mut owner, "validate", &nonce, deadline)?,
            &reservation,
        )?;
        invalid.check()?;
        worker.send("bind")?;
        let uia = worker.expect("uia-bound", deadline)?;
        check_channel(&uia, &channel)?;
        if uia["root_hwnd"] != reservation.handle {
            return Err(protocol("UIA root does not match the reserved fixture"));
        }
        let root: NativeNode = serde_json::from_value(uia["root"].clone()).map_err(protocol)?;
        let title = root
            .name
            .as_ref()
            .filter(|name| **name == fixture.title())
            .ok_or_else(|| protocol("unexpected fixture title"))?
            .clone();
        let frame = root
            .bounds()
            .ok_or_else(|| protocol("fixture selection bounds unavailable"))?;
        check_reservation(
            &owner_request(&mut owner, "validate", &nonce, deadline)?,
            &reservation,
        )?;
        invalid.check()?;
        worker.send("continue-wgc")?;
        let bound = worker.expect("bound", deadline)?;
        check_channel(&bound, &channel)?;
        if bound["dpi"] != 96 {
            return Err(protocol("fixture geometry is only verified at 96 DPI"));
        }
        let mut selector = Endpoint::spawn(
            &binary(binaries, "selector"),
            scratch.path(),
            &[fixture.name()],
            None,
        )?;
        let approved = selector.expect("selection-response", deadline)?["approved"]
            .as_bool()
            .ok_or_else(|| protocol("invalid fixture confirmation reply"))?;
        let status = selector
            .wait_until(deadline)?
            .ok_or_else(|| protocol("confirmation process did not close"))?;
        if status.code() != Some(if approved { 0 } else { 1 }) {
            return Err(protocol(
                "fixture confirmation exit did not match the response",
            ));
        }
        if !approved {
            crate::selection_cleanup::close_cancelled(
                || worker.close().map(|_| ()),
                || owner.close().map(|_| ()),
            )?;
            return Ok(None);
        }
        check_reservation(
            &owner_request(&mut owner, "validate", &nonce, deadline)?,
            &reservation,
        )?;
        invalid.check()?;
        let target = TargetAuthority {
            operation_id,
            receipt: Uuid::new_v4().try_into().map_err(protocol)?,
        };
        let selected = SelectedWindow {
            // The shared legacy display fields use fixture labels. The native PID/HWND never leave the adapter.
            identity: WindowIdentity {
                window_id: fixture.ordinal(),
                bundle_id: "com.lens.dev.win32-fixture".into(),
                pid: 0,
            },
            facts: WindowObservableFacts {
                title,
                application_name: "Lens Win32 fixture".into(),
                frame,
            },
        };
        Ok(Some(Self {
            worker,
            owner,
            scratch,
            invalid,
            observing: Arc::default(),
            selected,
            target,
            channel,
            reservation,
            next_read: 1,
            sample: None,
        }))
    }

    fn verify_owner(&mut self, verb: &str, deadline: Instant) -> Result<(), PlatformError> {
        self.invalid.check()?;
        let result = owner_request(&mut self.owner, verb, &self.reservation.nonce, deadline)
            .and_then(|reply| check_reservation(&reply, &self.reservation));
        if result.is_err() {
            self.invalid.close();
        }
        result?;
        self.invalid.check()
    }

    fn read(
        &mut self,
        key: TargetReadKey,
        limits: ExtractionLimits,
        probe: &ReadProbe,
    ) -> Result<ExtractionResult, PlatformError> {
        let number = key.sequence.get();
        self.next_read += 1;
        self.sample = None;
        let budget = if matches!(probe.mode_for(key.sequence), FixtureReadProbe::Normal) {
            STEP
        } else {
            Duration::from_secs(25)
        };
        let deadline = Instant::now() + budget;
        self.verify_owner(if number == 1 { "validate" } else { "refresh" }, deadline)?;
        let read: NativeRead = serde_json::from_value(probe.read(
            &mut self.worker,
            &self.channel,
            key.sequence,
            deadline,
        )?)
        .map_err(protocol)?;
        self.invalid.check()?;
        if read.channel != self.channel || read.read != key.sequence {
            return Err(protocol("fixture worker result correlation mismatch"));
        }
        let extent = PixelExtent {
            width: read.width,
            height: read.height,
        };
        if !extent.is_valid()
            || extent.width > 1024
            || extent.height > 1024
            || read.dwm_before_hr != 0
            || read.dwm_after_hr != 0
            || read.dwm_before != read.dwm_after
        {
            return Err(protocol("unverified fixture frame geometry"));
        }
        let desktop =
            bounds(read.dwm_before).ok_or_else(|| protocol("invalid fixture desktop bounds"))?;
        let frame = AcquiredFrame {
            id: Uuid::new_v4().try_into().map_err(protocol)?,
            target: CaptureTarget::Registered(key),
            extent,
            content_bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: f64::from(extent.width),
                height: f64::from(extent.height),
            },
            desktop_mapping: DesktopMapping::Known {
                space: DesktopSpace::DesktopPhysicalPixels,
                desktop_bounds: desktop,
                scale_x: 1.0,
                scale_y: 1.0,
            },
        };
        if !frame.is_valid() {
            return Err(protocol(
                "fixture frame extent did not match native metadata",
            ));
        }
        let mut png = Vec::new();
        File::open(self.scratch.path().join(format!("read-{number}.png")))
            .map_err(protocol)?
            .take(PNG_LIMIT as u64 + 1)
            .read_to_end(&mut png)
            .map_err(protocol)?;
        if png.len() > PNG_LIMIT {
            return Err(protocol("fixture PNG exceeded transfer budget"));
        }
        let mut reader =
            image::ImageReader::with_format(Cursor::new(&png), image::ImageFormat::Png);
        let mut decode_limits = image::Limits::default();
        decode_limits.max_image_width = Some(extent.width);
        decode_limits.max_image_height = Some(extent.height);
        decode_limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(decode_limits);
        let decoded = reader.decode().map_err(protocol)?;
        if decoded.width() != extent.width || decoded.height() != extent.height {
            return Err(protocol(
                "fixture PNG dimensions did not match the acquired frame",
            ));
        }
        let mut facts = self.selected.facts.clone();
        facts.frame = desktop;
        let extraction = extraction(read.uia, facts, limits)?;
        self.verify_owner("validate", deadline)?;
        self.sample = Some(Sample {
            frame,
            png: Some((png, decoded)),
            transferred: false,
            committed: false,
        });
        Ok(extraction)
    }

    fn sample(&mut self, key: TargetReadKey) -> Result<&mut Sample, PlatformError> {
        self.sample
            .as_mut()
            .filter(|sample| sample.frame.target == CaptureTarget::Registered(key))
            .ok_or_else(|| protocol("missing paired fixture read for this key"))
    }

    fn close(&mut self) -> Result<(), PlatformError> {
        self.invalid.close();
        // Keep the target reserved until the acquisition worker has closed.
        let _ = self.worker.send("stop");
        let worker = self.worker.close();
        if worker.is_ok() && self.owner.wait_until(Instant::now())?.is_none() {
            let _ = owner_request(
                &mut self.owner,
                "release",
                &self.reservation.nonce,
                Instant::now() + STEP,
            );
            let _ = self.owner.send("stop");
        }
        let owner = self.owner.close();
        eprintln!("fixture-cleanup worker={worker:?} owner={owner:?}");
        worker.and(owner).map(|_| ())
    }
}

fn check_channel(value: &Value, channel: &str) -> Result<(), PlatformError> {
    if value["channel"] == channel {
        Ok(())
    } else {
        Err(protocol("fixture worker channel mismatch"))
    }
}

fn extraction(
    tree: NativeTree,
    facts: WindowObservableFacts,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    if tree.children.len() > 8 || tree.root.name.as_deref() != Some(&facts.title) {
        return Err(protocol("fixture tree did not match retained root"));
    }
    let mut metrics = ExtractionMetrics {
        truncated_nodes: tree.truncated,
        ..Default::default()
    };
    let mut nodes: Vec<ExtractedNode> = Vec::new();
    for (order, native) in std::iter::once(Some(tree.root))
        .chain(tree.children)
        .enumerate()
    {
        if nodes.len() == limits.max_nodes as usize {
            metrics.truncated_nodes = true;
            break;
        }
        let Some(native) = native else {
            metrics.children_read_errors += 1;
            continue;
        };
        let id = format!("node-{order:06}");
        let native_bounds = native.bounds();
        let title = native.name.filter(|value| {
            if metrics.text_bytes + value.len() > limits.max_text_bytes as usize {
                metrics.truncated_text = true;
                false
            } else {
                metrics.text_bytes += value.len();
                true
            }
        });
        if order != 0 {
            nodes[0].children.push(id.clone());
        }
        nodes.push(ExtractedNode {
            semantic_role: match (native.role_hr, native.role) {
                (0, 50020) => SemanticRole::Text,
                (0, 50006) => SemanticRole::Image,
                _ => SemanticRole::Unknown,
            },
            purpose: if order == 0 {
                NodePurpose::WindowRoot
            } else {
                NodePurpose::Content
            },
            id,
            parent_id: (order != 0).then(|| "node-000000".into()),
            order,
            depth: usize::from(order != 0),
            role: (native.role_hr == 0).then(|| native.role.to_string()),
            subrole: None,
            title,
            value: None,
            description: None,
            bounds: native_bounds,
            resource_refs: vec![],
            children: vec![],
        });
    }
    metrics.visited_nodes = nodes.len();
    let text = nodes
        .iter()
        .skip(1)
        .filter_map(|node| node.title.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let quality =
        if metrics.truncated_nodes || metrics.truncated_text || metrics.children_read_errors != 0 {
            ExtractionQuality::Partial
        } else {
            ExtractionQuality::Full
        };
    Ok(ExtractionResult {
        source_api: SourceApi::WindowsUia,
        bounds_space: Some(DesktopSpace::DesktopPhysicalPixels),
        quality,
        resolved_window: Some(ResolvedWindow {
            facts,
            resolution_score: 1.0,
        }),
        nodes,
        text,
        metrics,
        diagnostics: vec![],
    })
}

impl Accessibility for FixturePlatform {
    fn extract(
        &self,
        target: ExtractionTarget,
        limits: ExtractionLimits,
    ) -> Result<ExtractionResult, PlatformError> {
        let ExtractionTarget::Registered(key) = target else {
            return Err(protocol(
                "the Windows development fixture has no Legacy route",
            ));
        };
        let mut slot = self.slot()?;
        let registration = current(&mut slot, key)?;
        if key.sequence.get() != registration.next_read
            || registration.next_read > self.read_limit
            || registration
                .sample
                .as_ref()
                .is_some_and(|sample| !sample.committed)
        {
            return Err(protocol("fixture read is out of sequence or over budget"));
        }
        let result = registration.read(key, limits, &self.probe);
        if result.is_err() {
            registration.invalid.close();
        }
        result
    }
}

impl Capture for FixturePlatform {
    fn capture(
        &self,
        target: CaptureTarget,
        requests: &[CaptureRequest],
        limits: ImageCaptureLimits,
    ) -> Result<CaptureBatch, PlatformError> {
        let CaptureTarget::Registered(key) = target else {
            return Err(protocol(
                "the Windows development fixture has no Legacy route",
            ));
        };
        let mut slot = self.slot()?;
        let registration = current(&mut slot, key)?;
        let sample = registration.sample(key)?;
        let (png, decoded) = sample
            .png
            .take()
            .ok_or_else(|| protocol("fixture frame already consumed"))?;
        let mut batch = CaptureBatch::default();
        let extent = sample.frame.extent;
        let mut total_bytes = 0usize;
        for request in requests {
            let source = match request.region {
                CaptureRegion::FrameContent => Some(sample.frame.content_bounds),
                CaptureRegion::DesktopRegion {
                    space,
                    bounds,
                    observed_window_frame,
                } => match sample.frame.desktop_mapping {
                    DesktopMapping::Known { desktop_bounds, .. }
                        if desktop_bounds == observed_window_frame =>
                    {
                        sample.frame.desktop_to_frame(space, bounds)
                    }
                    _ => None,
                },
            };
            let region = source.and_then(|source| {
                let clipped = sample.frame.clip(source)?;
                let crop = PixelRect::covering(clipped, extent)?;
                Some((source, clipped, crop))
            });
            let Some((source, clipped, crop)) = region else {
                batch.omissions.push(CaptureOmission {
                    attachment_id: request.id.clone(),
                    reason: CaptureOmissionReason::GeometryUnavailable,
                    detail: "fixture region lacks a matching frame transform".into(),
                });
                continue;
            };
            let exceeds_extent = crop.width.max(crop.height) > limits.max_long_edge
                || u64::from(crop.width) * u64::from(crop.height) > u64::from(limits.max_pixels);
            let bytes = if exceeds_extent {
                None
            } else if crop.x == 0
                && crop.y == 0
                && crop.width == extent.width
                && crop.height == extent.height
            {
                Some(png.clone())
            } else {
                let mut buffer = Cursor::new(Vec::new());
                decoded
                    .crop_imm(crop.x, crop.y, crop.width, crop.height)
                    .write_to(&mut buffer, image::ImageFormat::Png)
                    .map_err(protocol)?;
                Some(buffer.into_inner())
            };
            let Some(bytes) = bytes.filter(|b| {
                b.len() <= limits.max_attachment_bytes as usize
                    && b.len().saturating_add(total_bytes) <= limits.max_total_bytes as usize
            }) else {
                batch.omissions.push(CaptureOmission {
                    attachment_id: request.id.clone(),
                    reason: CaptureOmissionReason::ByteBudget,
                    detail: "fixture crop exceeded the image budget".into(),
                });
                continue;
            };
            total_bytes += bytes.len();
            batch.captures.push(CapturedImage {
                attachment_id: request.id.clone(),
                frame_id: sample.frame.id,
                source_bounds: source,
                captured_bounds: clipped,
                crop,
                coverage: if source == clipped {
                    CaptureCoverage::FullRegion
                } else {
                    CaptureCoverage::VisibleSubregion
                },
                pixel_width: crop.width as usize,
                pixel_height: crop.height as usize,
                png: bytes,
            });
        }
        if !batch.captures.is_empty() {
            batch.frame = Some(sample.frame.clone());
        }
        sample.transferred = !batch.captures.is_empty();
        registration.invalid.check()?;
        Ok(batch)
    }
}

struct ObservationHandle {
    open: Arc<AtomicBool>,
    invalid: Arc<Invalidation>,
}

impl ObservationRegistration for ObservationHandle {
    fn close(&mut self) -> Result<(), PlatformError> {
        self.open.store(false, Ordering::Release);
        self.invalid.wake();
        Ok(())
    }
}

impl Drop for ObservationHandle {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct Events {
    open: Arc<AtomicBool>,
    invalid: Arc<Invalidation>,
    next: Option<WindowObservationEvent>,
}

impl ObservationEvents for Events {
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<WindowObservationEvent>> {
        self.invalid.register(cx);
        if !self.open.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        if self.invalid.closed() {
            return Poll::Ready(if self.invalid.destroyed() {
                self.next.take()
            } else {
                None
            });
        }
        Poll::Pending
    }
}

impl Observation for FixturePlatform {
    fn observe(&self, request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
        let slot = self.slot()?;
        let registration = slot
            .as_ref()
            .filter(|r| r.target == request.target)
            .ok_or_else(|| protocol("unknown fixture observation registration"))?;
        registration.invalid.check()?;
        if registration.observing.swap(true, Ordering::AcqRel) {
            return Err(protocol("fixture supports one observation subscription"));
        }
        let open = Arc::new(AtomicBool::new(true));
        let notification = WindowObservationNotification::WindowDestroyed;
        Ok(ObservationSession {
            registration: Box::new(ObservationHandle {
                open: open.clone(),
                invalid: registration.invalid.clone(),
            }),
            events: Box::new(Events {
                open: open.clone(),
                invalid: registration.invalid.clone(),
                next: Some(WindowObservationEvent {
                    target: request.target,
                    context_id: request.context_id,
                    source_registration_id: request.source_registration_id,
                    observer_epoch: request.observer_epoch,
                    notification,
                }),
            }),
            start: WindowObservationStart {
                registered_notifications: vec![notification],
                diagnostics: vec![],
            },
        })
    }
}

impl AccessibilityTrust for FixturePlatform {
    fn inspect(&self) -> bool {
        // Adapter-local readiness of the retained fixture, not an OS-wide UIA permission.
        self.slot()
            .is_ok_and(|slot| slot.as_ref().is_some_and(|r| !r.invalid.closed()))
    }

    fn request(&self) -> bool {
        self.inspect()
    }
}
