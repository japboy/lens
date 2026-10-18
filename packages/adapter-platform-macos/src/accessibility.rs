//! Reviewed native AX boundary: exact retained objects, CFEqual graph identity,
//! bounded attribute batches and main-run-loop observation share one owner.
//!
//! axuielement 0.10 exposes neither public element equality nor raw handles;
//! accessibility 0.3 exposes equality but lacks bounded attribute reads and
//! observation. Neither can preserve this subsystem's contract as a whole.
#![deny(unsafe_code)]

use port_platform::{
    model::{Bounds, ExtractionResult, SelectedWindow, WindowIdentity},
    observation::{
        ObservationEvents, ObservationRegistration, ObservationRequest, ObservationSession,
        WindowObservationEvent, WindowObservationNotification, WindowObservationStart,
    },
    ExtractionLimits, PlatformError,
};
use serde::{de::DeserializeOwned, Deserialize};
use std::{
    ffi::{c_char, c_void, CStr, CString},
    num::NonZeroU64,
    ptr::NonNull,
    sync::Arc,
};
use tokio::sync::mpsc;
use uuid::Uuid;

type WindowObservationCallback = unsafe extern "C" fn(*const c_char, *mut c_void);

#[allow(
    unsafe_code,
    reason = "[boundary:ax-ffi] native AX retains exact objects shared by bounded extraction and observation; available safe crates cannot preserve all three contracts"
)]
unsafe extern "C" {
    fn lens_register_ax_window_for_operation(
        operation_id: *const c_char,
        window_id: u32,
        pid: i32,
        title: *const c_char,
        application_name: *const c_char,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> bool;
    fn lens_registered_ax_window_frame(
        operation_id: *const c_char,
        window_id: u32,
        x: *mut f64,
        y: *mut f64,
        width: *mut f64,
        height: *mut f64,
    ) -> i32;
    fn lens_extract_window_json(
        pid: i32,
        selected_title: *const c_char,
        application_name: *const c_char,
        selected_x: f64,
        selected_y: f64,
        selected_width: f64,
        selected_height: f64,
        max_nodes: u32,
        max_text_bytes: u32,
        max_resource_refs: u32,
        max_resource_uri_bytes: u32,
        max_total_resource_uri_bytes: u32,
    ) -> *mut c_char;
    fn lens_extract_registered_window_json(
        operation_id: *const c_char,
        window_id: u32,
        max_nodes: u32,
        max_text_bytes: u32,
        max_resource_refs: u32,
        max_resource_uri_bytes: u32,
        max_total_resource_uri_bytes: u32,
    ) -> *mut c_char;
    fn lens_start_window_observation_json(
        operation_id: *const c_char,
        context_id: *const c_char,
        source_registration_id: *const c_char,
        observer_epoch: u64,
        window_id: u32,
        callback: WindowObservationCallback,
        context: *mut c_void,
        started_out: *mut bool,
    ) -> *mut c_char;
    fn lens_stop_window_observation(
        operation_id: *const c_char,
        source_registration_id: *const c_char,
        expected_context: *mut c_void,
    ) -> bool;
    fn lens_release_registered_window(operation_id: *const c_char, window_id: u32) -> bool;
    fn lens_release_window_operation(operation_id: *const c_char) -> bool;
    fn lens_free_string(value: *mut c_char);
}

fn uuid_text(value: Uuid) -> CString {
    CString::new(value.to_string()).expect("UUID text contains no interior NUL")
}

fn target_text(value: &str, field: &str) -> Result<CString, PlatformError> {
    CString::new(value)
        .map_err(|_| PlatformError::Operation(format!("{field} contains an interior NUL")))
}

fn finite_bounds(bounds: &Bounds) -> bool {
    [bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f64::is_finite)
}

// Constructed only from malloc-owned responses returned by the declarations above.
// Retaining this owner until parsing finishes guarantees the matching native free
// on success, JSON failure and unwind; no borrowed native JSON escapes this module.
struct NativeJson {
    pointer: NonNull<c_char>,
    release: fn(NonNull<c_char>),
}

impl NativeJson {
    fn response(raw: *mut c_char, operation: &str) -> Result<Self, PlatformError> {
        Self::response_with_release(raw, operation, release_native_json)
    }

    // The effect is an allocator pairing, not an alternate decoder/owner. Tests
    // retain real CString allocations while exercising this same parse and Drop.
    fn response_with_release(
        raw: *mut c_char,
        operation: &str,
        release: fn(NonNull<c_char>),
    ) -> Result<Self, PlatformError> {
        NonNull::new(raw)
            .map(|pointer| Self { pointer, release })
            .ok_or_else(|| PlatformError::Operation(format!("{operation} returned no data")))
    }

    fn parse<T: DeserializeOwned>(&self) -> Result<T, PlatformError> {
        let json = {
            #[allow(
                unsafe_code,
                reason = "[boundary:native-json] decode the owned NUL-terminated response from the retained native AX subsystem"
            )]
            // SAFETY: NativeJson owns one live, NUL-terminated native response. The
            // borrow ends before Drop invokes the allocator's matching free function.
            unsafe { CStr::from_ptr(self.pointer.as_ptr()) }
        }
        .to_string_lossy();
        serde_json::from_str(&json)
            .map_err(|error| PlatformError::InvalidResponse(format!("{error}; response={json}")))
    }
}

impl Drop for NativeJson {
    fn drop(&mut self) {
        (self.release)(self.pointer);
    }
}

fn release_native_json(pointer: NonNull<c_char>) {
    #[allow(
        unsafe_code,
        reason = "[boundary:native-json] release the owned native AX JSON buffer with its matching allocator"
    )]
    // SAFETY: Production NativeJson values uniquely own a LensCopyJSONString
    // allocation; Drop invokes this matching allocator exactly once.
    unsafe {
        lens_free_string(pointer.as_ptr())
    };
}

pub(crate) fn register_target(
    operation_id: Uuid,
    target: &SelectedWindow,
) -> Result<(), PlatformError> {
    if !finite_bounds(&target.facts.frame) {
        return Err(PlatformError::Operation(
            "selected window has non-finite bounds".into(),
        ));
    }
    let operation = uuid_text(operation_id);
    let title = target_text(&target.facts.title, "window title")?;
    let application = target_text(&target.facts.application_name, "application name")?;
    let bounds = &target.facts.frame;
    let registered = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-ffi] register picker metadata with the exact native AX owner without reconstructing handles or weakening object identity"
        )]
        // SAFETY: Strings remain alive until native synchronously copies their
        // metadata on main; no SCWindow or AX pointer crosses this boundary.
        unsafe {
            lens_register_ax_window_for_operation(
                operation.as_ptr(),
                target.identity.window_id,
                target.identity.pid,
                title.as_ptr(),
                application.as_ptr(),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            )
        }
    };
    if registered {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "native selected-window registration is unavailable".into(),
        ))
    }
}

pub(crate) fn current_bounds(
    operation_id: Uuid,
    window_id: u32,
) -> Result<Option<Bounds>, PlatformError> {
    let operation = uuid_text(operation_id);
    let mut bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };
    let found = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] read current geometry from the same promoted AXWindow used by extraction and observation"
        )]
        // SAFETY: Four distinct initialized outputs and the UUID remain valid until
        // native finishes reading the exact retained AXWindow's current geometry.
        unsafe {
            lens_registered_ax_window_frame(
                operation.as_ptr(),
                window_id,
                &mut bounds.x,
                &mut bounds.y,
                &mut bounds.width,
                &mut bounds.height,
            )
        }
    };
    current_bounds_reply(found, bounds)
}

fn current_bounds_reply(status: i32, bounds: Bounds) -> Result<Option<Bounds>, PlatformError> {
    match status {
        // Only a never-promoted source may use its retained SCWindow frame.
        0 => Ok(None),
        1 if finite_bounds(&bounds) => Ok(Some(bounds)),
        _ => Err(PlatformError::Operation(
            "the promoted AXWindow did not expose finite current bounds".into(),
        )),
    }
}

pub(crate) fn extract_window(
    target: &SelectedWindow,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    if !finite_bounds(&target.facts.frame) {
        return Err(PlatformError::Operation(
            "selected window has non-finite bounds".into(),
        ));
    }
    let title = target_text(&target.facts.title, "window title")?;
    let application = target_text(&target.facts.application_name, "application name")?;
    let bounds = &target.facts.frame;
    let raw = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-ffi] bounded native AX extraction preserves CFEqual graph deduplication and typed attributed-text/URI reads unavailable together in safe crates"
        )]
        // SAFETY: Strings live through this synchronous call; finite scalar metadata
        // and explicit limits are copied, and native returns an owned JSON buffer.
        unsafe {
            lens_extract_window_json(
                target.identity.pid,
                title.as_ptr(),
                application.as_ptr(),
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                limits.max_nodes,
                limits.max_text_bytes,
                limits.max_resource_refs,
                limits.max_resource_uri_bytes,
                limits.max_total_resource_uri_bytes,
            )
        }
    };
    NativeJson::response(raw, "native Accessibility extraction")?.parse()
}

pub(crate) fn extract_registered_window(
    operation_id: Uuid,
    identity: &WindowIdentity,
    limits: ExtractionLimits,
) -> Result<ExtractionResult, PlatformError> {
    let operation = uuid_text(operation_id);
    let raw = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-ffi] extract from the exact promoted AXWindow rather than heuristically resolving a replacement through a separate safe wrapper"
        )]
        // SAFETY: UUID storage remains alive during the blocking call. Native uses
        // its retained operation/window object and returns a unique owned response.
        unsafe {
            lens_extract_registered_window_json(
                operation.as_ptr(),
                identity.window_id,
                limits.max_nodes,
                limits.max_text_bytes,
                limits.max_resource_refs,
                limits.max_resource_uri_bytes,
                limits.max_total_resource_uri_bytes,
            )
        }
    };
    NativeJson::response(raw, "registered native Accessibility extraction")?.parse()
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum NativeObservationStartReply {
    Started {
        registered_notifications: Vec<WindowObservationNotification>,
        diagnostics: Vec<String>,
    },
    Error {
        message: String,
    },
}

struct CallbackContext {
    operation_id: Uuid,
    context_id: Uuid,
    source_registration_id: Uuid,
    observer_epoch: NonZeroU64,
    window_id: u32,
    sender: mpsc::Sender<WindowObservationEvent>,
}

fn dispatch_event(json: &str, context: &CallbackContext) {
    let Ok(event) = serde_json::from_str::<WindowObservationEvent>(json) else {
        return;
    };
    if event.operation_id != context.operation_id
        || event.context_id != context.context_id
        || event.source_registration_id != context.source_registration_id
        || event.observer_epoch != context.observer_epoch
        || event.window_id != context.window_id
    {
        return;
    }
    // Capacity one deliberately coalesces invalidations while preserving authority.
    let _ = context.sender.try_send(event);
}

/// Deliver a borrowed native AX invalidation to its registration.
///
/// # Safety
/// Non-null `json` must point to a NUL-terminated buffer valid for this call.
/// Non-null `context` must borrow the registration's live `Arc<CallbackContext>`
/// allocation. Call on the main run loop before synchronous native stop clears
/// the callback and refcon; never invoke this callback after the owner is dropped.
#[allow(
    unsafe_code,
    reason = "[boundary:ax-observer] C AXObserver callback receives borrowed JSON and a registration-owned context whose lifetime ends only after synchronous stop"
)]
unsafe extern "C" fn observation_callback(json: *const c_char, context: *mut c_void) {
    if json.is_null() || context.is_null() {
        return;
    }
    let context = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] borrow the registration-owned AXObserver callback context while native guarantees it remains live"
        )]
        // SAFETY: Native borrows this shared Arc allocation until stop removes the run-loop
        // observer and clears its callback/refcon synchronously on main.
        unsafe {
            &*context.cast::<CallbackContext>()
        }
    };
    let borrowed_json = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] borrow native callback JSON only for the duration of its synchronous delivery"
        )]
        // SAFETY: The bridge retains the NUL-terminated buffer throughout callback.
        unsafe {
            CStr::from_ptr(json)
        }
    };
    let Ok(json) = borrowed_json.to_str() else {
        return;
    };
    dispatch_event(json, context);
}

struct NativeObservationRegistration {
    operation_id: Uuid,
    source_registration_id: Uuid,
    callback_context: Option<Arc<CallbackContext>>,
    stop_effect: fn(&mut NativeObservationRegistration) -> Result<(), PlatformError>,
}

impl NativeObservationRegistration {
    fn stop_inner(&mut self) -> Result<(), PlatformError> {
        if self.callback_context.is_none() {
            return Ok(());
        }
        (self.stop_effect)(self)
    }

    fn stop_with(&mut self, stop: impl FnOnce() -> bool) -> Result<(), PlatformError> {
        if self.callback_context.is_none() {
            return Ok(());
        }
        let stopped = stop();
        // A missing native source has already removed its observer/refcon.
        // Release the borrowed context only after the stop effect returns.
        self.callback_context = None;
        if stopped {
            Ok(())
        } else {
            Err(PlatformError::Operation(
                "native source observation was unavailable during stop".into(),
            ))
        }
    }
}

fn stop_native_observation(
    registration: &mut NativeObservationRegistration,
) -> Result<(), PlatformError> {
    crate::source::on_main(|| {
        let operation = uuid_text(registration.operation_id);
        let source = uuid_text(registration.source_registration_id);
        let expected_context = registration
            .callback_context
            .as_ref()
            .map_or(std::ptr::null_mut(), |context| {
                Arc::as_ptr(context).cast_mut().cast()
            });
        registration.stop_with(|| {
            #[allow(
                unsafe_code,
                reason = "[boundary:ax-observer] stop only the native observer borrowing this exact live Arc context before releasing its owner"
            )]
            // SAFETY: UUIDs and the shared Arc remain alive through synchronous teardown.
            // Native compares this receipt before touching the current registration, so
            // an older session cannot stop a new owner that reused its public IDs.
            // Shared stop_with drops this Arc only after the native effect returns.
            unsafe {
                lens_stop_window_observation(
                    operation.as_ptr(),
                    source.as_ptr(),
                    expected_context,
                )
            }
        })
    })
}

impl Drop for NativeObservationRegistration {
    fn drop(&mut self) {
        let _ = self.stop_inner();
        // An already-absent native source cannot retain its old callback/refcon.
        self.callback_context = None;
    }
}

impl ObservationRegistration for NativeObservationRegistration {
    fn close(&mut self) -> Result<(), PlatformError> {
        self.stop_inner()
    }
}

struct NativeObservationEvents(mpsc::Receiver<WindowObservationEvent>);
impl ObservationEvents for NativeObservationEvents {
    fn poll_next(
        &mut self,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<WindowObservationEvent>> {
        self.0.poll_recv(context)
    }
}

pub(crate) fn observe(request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
    // Native start performs lookup, promotion and observer installation. Keep
    // every step in one transaction with source registration/release so release
    // cannot be followed by installation on an already-detached retained source.
    crate::source::on_main(move || start_observation(request))
}

fn start_observation(request: ObservationRequest) -> Result<ObservationSession, PlatformError> {
    let operation = uuid_text(request.operation_id);
    let context = uuid_text(request.context_id);
    let source = uuid_text(request.source_registration_id);
    let (sender, receiver) = mpsc::channel(1);
    let callback_context = Arc::new(CallbackContext {
        operation_id: request.operation_id,
        context_id: request.context_id,
        source_registration_id: request.source_registration_id,
        observer_epoch: request.observer_epoch,
        window_id: request.identity.window_id,
        sender,
    });
    // The callback only reads Context and uses Sender's internal synchronization.
    // Shared Arc provenance survives adoption/moves; deriving this from &mut a
    // Box would invalidate the borrowed pointer when the unique owner moves.
    let raw_context = Arc::as_ptr(&callback_context).cast_mut().cast();
    let mut started = false;
    let raw = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] install observation on the same retained AXWindow used for extraction with an explicit callback ownership handshake"
        )]
        // SAFETY: The Arc allocation has shared provenance and remains Rust-owned. Native installs
        // all observers before returning and writes whether it retained the refcon.
        unsafe {
            lens_start_window_observation_json(
                operation.as_ptr(),
                context.as_ptr(),
                source.as_ptr(),
                request.observer_epoch.get(),
                request.identity.window_id,
                observation_callback,
                raw_context,
                &mut started,
            )
        }
    };
    finish_observation_start(
        &request,
        callback_context,
        receiver,
        started,
        NativeJson::response(raw, "native source observation"),
        stop_native_observation,
    )
}

fn finish_observation_start(
    request: &ObservationRequest,
    callback_context: Arc<CallbackContext>,
    receiver: mpsc::Receiver<WindowObservationEvent>,
    started: bool,
    response: Result<NativeJson, PlatformError>,
    stop_effect: fn(&mut NativeObservationRegistration) -> Result<(), PlatformError>,
) -> Result<ObservationSession, PlatformError> {
    // Establish cleanup ownership before response decoding: even a null or
    // malformed response after native installation must synchronously stop.
    let registration = NativeObservationRegistration {
        operation_id: request.operation_id,
        source_registration_id: request.source_registration_id,
        callback_context: started.then_some(callback_context),
        stop_effect,
    };
    let reply: NativeObservationStartReply = response?.parse()?;
    let start = match reply {
        NativeObservationStartReply::Started {
            registered_notifications,
            diagnostics,
        } => WindowObservationStart {
            registered_notifications,
            diagnostics,
        },
        NativeObservationStartReply::Error { message } => {
            return Err(PlatformError::Operation(message))
        }
    };
    if !started {
        return Err(PlatformError::InvalidResponse(
            "native observation reported started JSON without transferring ownership".into(),
        ));
    }
    Ok(ObservationSession {
        registration: Box::new(registration),
        events: Box::new(NativeObservationEvents(receiver)),
        start,
    })
}

pub(crate) fn release_registered_window(
    operation_id: Uuid,
    window_id: u32,
) -> Result<(), PlatformError> {
    let operation = uuid_text(operation_id);
    let released = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] release the exact native AX registration and synchronously remove its observer before discarding ownership"
        )]
        // SAFETY: UUID lives through this synchronous main-thread registry removal,
        // which also stops observation before dropping retained AX objects.
        unsafe {
            lens_release_registered_window(operation.as_ptr(), window_id)
        }
    };
    if released {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "native selected-window registration is unavailable".into(),
        ))
    }
}

pub(crate) fn release_window_operation(operation_id: Uuid) -> Result<(), PlatformError> {
    let operation = uuid_text(operation_id);
    let released = {
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] end the native AX operation and its observers as one ownership boundary"
        )]
        // SAFETY: UUID is valid until native removes this operation's AX observers
        // and exact retained objects synchronously on the main run loop.
        unsafe {
            lens_release_window_operation(operation.as_ptr())
        }
    };
    if released {
        Ok(())
    } else {
        Err(PlatformError::Operation(
            "native selected-window operation is unavailable".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    struct JsonAllocation {
        bytes: Arc<CString>,
        releases: Rc<Cell<usize>>,
    }

    thread_local! {
        static JSON_ALLOCATIONS: RefCell<Vec<JsonAllocation>> = const { RefCell::new(Vec::new()) };
        static STOP_PROBE: RefCell<Option<StopProbe>> = const { RefCell::new(None) };
    }

    fn release_test_json(pointer: NonNull<c_char>) {
        JSON_ALLOCATIONS.with(|allocations| {
            let mut allocations = allocations.borrow_mut();
            let index = allocations
                .iter()
                .position(|allocation| allocation.bytes.as_ptr() == pointer.as_ptr().cast_const())
                .expect("every response must release its unique matching allocation");
            let allocation = allocations.remove(index);
            allocation.releases.set(allocation.releases.get() + 1);
            drop(allocation.bytes);
        });
    }

    fn owned_test_json(text: &str) -> (Result<NativeJson, PlatformError>, Rc<Cell<usize>>) {
        // Retain the allocation through a shared owner before deriving the
        // pointer. Moving CString's unique inner owner afterward would retag it.
        let bytes = Arc::new(CString::new(text).unwrap());
        let pointer = bytes.as_ptr().cast_mut();
        let releases = Rc::new(Cell::new(0));
        JSON_ALLOCATIONS.with(|allocations| {
            allocations.borrow_mut().push(JsonAllocation {
                bytes,
                releases: releases.clone(),
            })
        });
        (
            NativeJson::response_with_release(
                pointer,
                "modeled native response",
                release_test_json,
            ),
            releases,
        )
    }

    #[derive(Debug, PartialEq, Eq)]
    enum StopEvent {
        StopWithLiveContext,
        ContextReleased,
    }

    struct StopProbe {
        sender: mpsc::WeakSender<WindowObservationEvent>,
        events: Rc<RefCell<Vec<StopEvent>>>,
        stop_result: bool,
    }

    fn stop_test_observation(
        registration: &mut NativeObservationRegistration,
    ) -> Result<(), PlatformError> {
        STOP_PROBE.with(|probe| {
            let probe = probe.borrow();
            let probe = probe.as_ref().expect("active modeled native stop effect");
            let result = registration.stop_with(|| {
                assert_eq!(
                    probe.sender.strong_count(),
                    1,
                    "stop must run before callback-context release"
                );
                probe
                    .events
                    .borrow_mut()
                    .push(StopEvent::StopWithLiveContext);
                probe.stop_result
            });
            assert_eq!(
                probe.sender.strong_count(),
                0,
                "shared teardown must release the context after stop"
            );
            probe.events.borrow_mut().push(StopEvent::ContextReleased);
            result
        })
    }

    struct ModeledStart {
        session: Result<ObservationSession, PlatformError>,
        events: Rc<RefCell<Vec<StopEvent>>>,
        weak_sender: mpsc::WeakSender<WindowObservationEvent>,
        context_pointer: *mut c_void,
    }

    fn modeled_start(
        started: bool,
        response: Result<NativeJson, PlatformError>,
        stop_result: bool,
    ) -> ModeledStart {
        let expected = event();
        let request = ObservationRequest {
            operation_id: expected.operation_id,
            context_id: expected.context_id,
            source_registration_id: expected.source_registration_id,
            observer_epoch: expected.observer_epoch,
            identity: WindowIdentity {
                window_id: expected.window_id,
                pid: 7,
                bundle_id: "test.application".into(),
            },
        };
        let (sender, receiver) = mpsc::channel(1);
        let weak_sender = sender.downgrade();
        let events = Rc::new(RefCell::new(Vec::new()));
        STOP_PROBE.with(|probe| {
            *probe.borrow_mut() = Some(StopProbe {
                sender: weak_sender.clone(),
                events: events.clone(),
                stop_result,
            })
        });
        let context = Arc::new(CallbackContext {
            operation_id: expected.operation_id,
            context_id: expected.context_id,
            source_registration_id: expected.source_registration_id,
            observer_epoch: expected.observer_epoch,
            window_id: expected.window_id,
            sender,
        });
        // Match production: native borrows the pointer before Rust adopts the
        // allocation into the registration returned by the safe start boundary.
        let pointer = Arc::as_ptr(&context).cast_mut().cast();
        let result = finish_observation_start(
            &request,
            context,
            receiver,
            started,
            response,
            stop_test_observation,
        );
        ModeledStart {
            session: result,
            events,
            weak_sender,
            context_pointer: pointer,
        }
    }

    #[test]
    fn miri_boundary_callback_borrows_live_json_and_context() {
        let expected = event();
        let (sender, mut receiver) = mpsc::channel(1);
        let context = Arc::new(CallbackContext {
            operation_id: expected.operation_id,
            context_id: expected.context_id,
            source_registration_id: expected.source_registration_id,
            observer_epoch: expected.observer_epoch,
            window_id: expected.window_id,
            sender,
        });
        let pointer = Arc::as_ptr(&context).cast_mut().cast();
        let valid = CString::new(serde_json::to_string(&expected).unwrap()).unwrap();
        let stale = WindowObservationEvent {
            observer_epoch: NonZeroU64::new(2).unwrap(),
            ..expected.clone()
        };
        let stale = CString::new(serde_json::to_string(&stale).unwrap()).unwrap();
        let malformed = CString::new("invalid JSON").unwrap();
        let invalid_utf8 = CString::new(vec![255]).unwrap();
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] exercise the production raw callback with live Rust-owned JSON and context allocations, including its explicit null-input cases"
        )]
        // SAFETY: The real callback only borrows each owned CString and stable
        // shared context during these synchronous calls; both outlive all calls.
        unsafe {
            observation_callback(valid.as_ptr(), pointer);
            observation_callback(valid.as_ptr(), pointer);
            observation_callback(stale.as_ptr(), pointer);
            observation_callback(malformed.as_ptr(), pointer);
            observation_callback(invalid_utf8.as_ptr(), pointer);
            observation_callback(std::ptr::null(), pointer);
            observation_callback(valid.as_ptr(), std::ptr::null_mut());
        }
        assert_eq!(receiver.try_recv().unwrap(), expected);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        drop(context);
        assert!(receiver.is_closed());

        // The pointer must also survive the real ownership-adoption path, not
        // just calls made while the owner stays in its original local variable.
        let (response, releases) = owned_test_json(
            r#"{"status":"started","registered_notifications":[],"diagnostics":[]}"#,
        );
        let ModeledStart {
            session,
            events,
            weak_sender: sender,
            context_pointer: pointer,
        } = modeled_start(true, response, true);
        let mut session = session.unwrap();
        #[allow(
            unsafe_code,
            reason = "[boundary:ax-observer] exercise the actual callback after production ownership adoption and before safe session close"
        )]
        // SAFETY: The successful production adoption now owns the exact context
        // whose address was borrowed before adoption; close has not run yet.
        unsafe {
            observation_callback(valid.as_ptr(), pointer)
        };
        let mut task = std::task::Context::from_waker(std::task::Waker::noop());
        assert_eq!(
            session.events.poll_next(&mut task),
            std::task::Poll::Ready(Some(expected))
        );
        session.registration.close().unwrap();
        drop(session);
        assert_eq!(sender.strong_count(), 0);
        assert_eq!(
            *events.borrow(),
            [StopEvent::StopWithLiveContext, StopEvent::ContextReleased]
        );
        assert_eq!(releases.get(), 1);
    }

    #[test]
    fn miri_boundary_json_valid_and_malformed_release_once() {
        for text in [
            r#"{"status":"error","message":"modeled refusal"}"#,
            "invalid JSON",
        ] {
            let (response, releases) = owned_test_json(text);
            let response = response.unwrap();
            let decoded = response.parse::<NativeObservationStartReply>();
            assert_eq!(decoded.is_ok(), text.starts_with('{'));
            assert_eq!(
                releases.get(),
                0,
                "parsing must not release a still-owned response"
            );
            drop(response);
            assert_eq!(releases.get(), 1);
        }
        JSON_ALLOCATIONS.with(|allocations| assert!(allocations.borrow().is_empty()));
    }

    #[test]
    fn miri_boundary_json_null_is_rejected_without_release() {
        assert!(NativeJson::response_with_release(
            std::ptr::null_mut(),
            "modeled allocation failure",
            release_test_json
        )
        .is_err());
        JSON_ALLOCATIONS.with(|allocations| assert!(allocations.borrow().is_empty()));
    }

    #[test]
    fn miri_boundary_observation_close_and_drop_stop_before_context_release() {
        for explicit_close in [false, true] {
            for stop_result in [false, true] {
                let (response, releases) = owned_test_json(
                    r#"{"status":"started","registered_notifications":[],"diagnostics":[]}"#,
                );
                let ModeledStart {
                    session,
                    events,
                    weak_sender: sender,
                    ..
                } = modeled_start(true, response, stop_result);
                let mut session = session.unwrap();
                assert_eq!(sender.strong_count(), 1);
                if explicit_close {
                    assert_eq!(session.registration.close().is_ok(), stop_result);
                    assert!(
                        session.registration.close().is_ok(),
                        "close must be idempotent"
                    );
                }
                drop(session);
                assert_eq!(sender.strong_count(), 0);
                assert_eq!(
                    *events.borrow(),
                    [StopEvent::StopWithLiveContext, StopEvent::ContextReleased]
                );
                assert_eq!(releases.get(), 1);
            }
        }
    }

    #[test]
    fn miri_boundary_observation_start_failure_stops_before_context_release() {
        for (started, text) in [
            (
                false,
                Some(r#"{"status":"error","message":"modeled refusal"}"#),
            ),
            (
                false,
                Some(r#"{"status":"started","registered_notifications":[],"diagnostics":[]}"#),
            ),
            (
                true,
                Some(r#"{"status":"error","message":"modeled late failure"}"#),
            ),
            (true, Some("invalid JSON")),
            (true, None),
        ] {
            let (response, releases) = match text {
                Some(text) => owned_test_json(text),
                None => (
                    NativeJson::response_with_release(
                        std::ptr::null_mut(),
                        "modeled allocation failure",
                        release_test_json,
                    ),
                    Rc::new(Cell::new(0)),
                ),
            };
            let ModeledStart {
                session,
                events,
                weak_sender: sender,
                ..
            } = modeled_start(started, response, true);
            assert!(session.is_err());
            assert_eq!(sender.strong_count(), 0);
            if started {
                assert_eq!(
                    *events.borrow(),
                    [StopEvent::StopWithLiveContext, StopEvent::ContextReleased]
                );
            } else {
                assert!(
                    events.borrow().is_empty(),
                    "a refused start never transfers stop authority"
                );
            }
            assert_eq!(releases.get(), usize::from(text.is_some()));
        }
    }

    #[test]
    fn miri_boundary_observation_cancel_drop_stops_before_context_release() {
        use std::future::Future;
        let (response, releases) = owned_test_json(
            r#"{"status":"started","registered_notifications":[],"diagnostics":[]}"#,
        );
        let ModeledStart {
            session,
            events,
            weak_sender: sender,
            ..
        } = modeled_start(true, response, true);
        let session = session.unwrap();
        let mut pending = Box::pin(async move {
            let _owned_session = session;
            std::future::pending::<()>().await;
        });
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(pending.as_mut().poll(&mut context).is_pending());
        assert_eq!(sender.strong_count(), 1);
        drop(pending);
        assert_eq!(sender.strong_count(), 0);
        assert_eq!(
            *events.borrow(),
            [StopEvent::StopWithLiveContext, StopEvent::ContextReleased]
        );
        assert_eq!(releases.get(), 1);
    }

    fn event() -> WindowObservationEvent {
        WindowObservationEvent {
            operation_id: Uuid::from_u128(1),
            context_id: Uuid::from_u128(2),
            source_registration_id: Uuid::from_u128(3),
            observer_epoch: NonZeroU64::MIN,
            window_id: 17,
            notification: WindowObservationNotification::WindowTitleChanged,
        }
    }

    #[test]
    fn observation_authority_filters_each_axis_and_coalesces_delivery() {
        let expected = event();
        let (sender, mut receiver) = mpsc::channel(1);
        let context = CallbackContext {
            operation_id: expected.operation_id,
            context_id: expected.context_id,
            source_registration_id: expected.source_registration_id,
            observer_epoch: expected.observer_epoch,
            window_id: expected.window_id,
            sender,
        };
        for stale in [
            WindowObservationEvent {
                operation_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                context_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                source_registration_id: Uuid::from_u128(99),
                ..expected.clone()
            },
            WindowObservationEvent {
                observer_epoch: NonZeroU64::new(2).unwrap(),
                ..expected.clone()
            },
            WindowObservationEvent {
                window_id: 18,
                ..expected.clone()
            },
        ] {
            dispatch_event(&serde_json::to_string(&stale).unwrap(), &context);
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
        dispatch_event("invalid JSON", &context);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        dispatch_event(&serde_json::to_string(&expected).unwrap(), &context);
        let later = WindowObservationEvent {
            notification: WindowObservationNotification::WindowResized,
            ..expected.clone()
        };
        dispatch_event(&serde_json::to_string(&later).unwrap(), &context);
        assert_eq!(receiver.try_recv().unwrap(), expected);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn event_stream_distinguishes_pending_delivery_and_terminal_close() {
        let (sender, receiver) = mpsc::channel(1);
        let mut events = NativeObservationEvents(receiver);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(events.poll_next(&mut context).is_pending());
        sender.try_send(event()).unwrap();
        assert_eq!(
            events.poll_next(&mut context),
            std::task::Poll::Ready(Some(event()))
        );
        assert!(events.poll_next(&mut context).is_pending());
        drop(sender);
        assert_eq!(events.poll_next(&mut context), std::task::Poll::Ready(None));
        assert_eq!(events.poll_next(&mut context), std::task::Poll::Ready(None));
    }

    #[test]
    fn ffi_geometry_rejects_non_finite_values_in_every_component() {
        let valid = Bounds {
            x: -20.0,
            y: 30.0,
            width: 100.0,
            height: 50.0,
        };
        assert!(finite_bounds(&valid));
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for component in 0..4 {
                let mut bounds = valid;
                match component {
                    0 => bounds.x = invalid,
                    1 => bounds.y = invalid,
                    2 => bounds.width = invalid,
                    _ => bounds.height = invalid,
                }
                assert!(!finite_bounds(&bounds));
            }
        }
        assert!(target_text("app\0title", "window title").is_err());
        assert_eq!(
            target_text("\u{65e5}\u{672c}\u{8a9e}", "window title")
                .unwrap()
                .to_str()
                .unwrap(),
            "\u{65e5}\u{672c}\u{8a9e}"
        );
    }

    #[test]
    fn observation_registration_and_events_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<NativeObservationRegistration>();
        assert_send::<NativeObservationEvents>();
        assert_send::<ObservationSession>();
    }

    #[test]
    fn current_frame_fallback_is_limited_to_never_promoted_sources() {
        let valid = Bounds {
            x: 1.0,
            y: 2.0,
            width: 30.0,
            height: 40.0,
        };
        assert_eq!(current_bounds_reply(1, valid).unwrap(), Some(valid));
        assert_eq!(current_bounds_reply(0, valid).unwrap(), None);
        assert!(current_bounds_reply(-1, valid).is_err());
        assert!(current_bounds_reply(2, valid).is_err());
        assert!(current_bounds_reply(
            1,
            Bounds {
                x: f64::NAN,
                ..valid
            }
        )
        .is_err());
    }
}
