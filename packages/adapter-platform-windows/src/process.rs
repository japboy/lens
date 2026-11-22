//! Bounded, private stdio for the three fixed development binaries.

use port_platform::PlatformError;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc, Mutex,
    },
    task::{Context, Waker},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LINE_LIMIT: u64 = 16 * 1024;
const STREAM_LIMIT: usize = 128 * 1024;
const CLEANUP: Duration = Duration::from_secs(2);

pub(crate) fn protocol(message: impl ToString) -> PlatformError {
    PlatformError::InvalidResponse(message.to_string())
}

#[derive(Default)]
pub(crate) struct Invalidation {
    closed: AtomicBool,
    destroyed: AtomicBool,
    subscriber: Mutex<Option<Waker>>,
}

impl Invalidation {
    pub fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn destroyed(&self) -> bool {
        self.destroyed.load(Ordering::Acquire)
    }

    pub fn check(&self) -> Result<(), PlatformError> {
        if self.closed() {
            Err(protocol("fixture authority lost; select a new fixture"))
        } else {
            Ok(())
        }
    }

    pub fn register(&self, cx: &Context<'_>) {
        if let Ok(mut subscriber) = self.subscriber.lock() {
            *subscriber = Some(cx.waker().clone());
        }
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.wake();
    }

    pub fn wake(&self) {
        let waker = self.subscriber.lock().ok().and_then(|mut slot| slot.take());
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub(crate) struct Endpoint {
    child: Child,
    input: Option<ChildStdin>,
    events: Receiver<Value>,
    exit_reported: bool,
}

impl Endpoint {
    pub fn spawn(
        bin: &Path,
        cwd: &Path,
        args: &[&str],
        invalidation: Option<Arc<Invalidation>>,
    ) -> Result<Self, PlatformError> {
        let label = bin
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("fixture");
        let mut child = Command::new(bin)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(protocol)?;
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (sender, events) = mpsc::sync_channel(16);
        let pid = child.id();
        eprintln!("fixture-child launch role={label} pid={pid}");
        let label = label.to_owned();
        let endpoint = Self {
            child,
            input,
            events,
            exit_reported: false,
        };
        let stderr_signal = invalidation.clone();
        thread::Builder::new()
            .name("lens-win32-fixture-stderr".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                if stderr
                    .take(STREAM_LIMIT as u64 + 1)
                    .read_to_end(&mut bytes)
                    .is_err()
                    || bytes.len() > STREAM_LIMIT
                {
                    if let Some(signal) = stderr_signal {
                        signal.close();
                    }
                }
                if !bytes.is_empty() {
                    eprintln!(
                        "fixture-child stderr pid={pid} {:?}",
                        String::from_utf8_lossy(&bytes)
                    );
                }
            })
            .map_err(protocol)?;
        thread::Builder::new()
            .name("lens-win32-fixture-stdout".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut bytes = 0;
                loop {
                    let mut line = Vec::new();
                    let Ok(count) = (&mut reader)
                        .take(LINE_LIMIT + 1)
                        .read_until(b'\n', &mut line)
                    else {
                        break;
                    };
                    bytes += count;
                    if count == 0
                        || line.last() != Some(&b'\n')
                        || count as u64 > LINE_LIMIT
                        || bytes > STREAM_LIMIT
                    {
                        break;
                    }
                    let Ok(record) = serde_json::from_slice::<Value>(&line) else {
                        break;
                    };
                    eprintln!("fixture-child record role={label} pid={pid} {record}");
                    if matches!(
                        record["event"].as_str(),
                        Some(
                            "native-destroy"
                                | "wgc-closed"
                                | "uia-failure"
                                | "worker-error"
                                | "read-blocked"
                        )
                    ) {
                        if let Some(signal) = &invalidation {
                            if record["event"] == "native-destroy" {
                                signal.destroyed.store(true, Ordering::Release);
                            }
                            signal.close();
                        }
                    }
                    if sender.try_send(record).is_err() {
                        break;
                    }
                }
                if let Some(signal) = &invalidation {
                    signal.close();
                }
            })
            .map_err(protocol)?;
        Ok(endpoint)
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn send(&mut self, command: &str) -> Result<(), PlatformError> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| protocol("child stdin is closed"))?;
        writeln!(input, "{command}").map_err(protocol)
    }

    pub fn expect(&self, event: &str, deadline: Instant) -> Result<Value, PlatformError> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(protocol(format!("fixture deadline awaiting {event}")));
            }
            match self.events.recv_timeout(remaining) {
                Ok(value) if value["event"] == event => return Ok(value),
                Ok(value)
                    if event == "selection-response" && value["event"] == "selector-error" =>
                {
                    return Err(protocol(format!("fixture confirmation failed: {value}")))
                }
                Ok(value)
                    if matches!(
                        value["event"].as_str(),
                        Some(
                            "native-destroy"
                                | "wgc-closed"
                                | "uia-failure"
                                | "worker-error"
                                | "read-blocked"
                        )
                    ) =>
                {
                    return Err(protocol(format!("fixture authority lost: {value}")))
                }
                Ok(value) if matches!(value["event"].as_str(), Some("capture-started")) => continue,
                Ok(_) => {
                    return Err(protocol(format!(
                        "unexpected fixture reply awaiting {event}"
                    )))
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(protocol(format!("fixture deadline awaiting {event}")))
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(protocol("fixture channel closed"))
                }
            }
        }
    }

    pub fn wait_until(&mut self, deadline: Instant) -> Result<Option<ExitStatus>, PlatformError> {
        loop {
            if let Some(status) = self.child.try_wait().map_err(protocol)? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn close(&mut self) -> Result<ExitStatus, PlatformError> {
        self.input.take();
        if let Ok(Some(status)) = self.wait_until(Instant::now() + Duration::from_millis(250)) {
            self.record_exit(status, false);
            return Ok(status);
        }
        self.terminate()
    }

    fn record_exit(&mut self, status: ExitStatus, forced: bool) {
        if !self.exit_reported {
            self.exit_reported = true;
            let observed_unix_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            eprintln!(
                "fixture-child exit pid={} forced={forced} status={status} observed_unix_ms={observed_unix_ms}",
                self.id()
            );
        }
    }

    pub fn terminate(&mut self) -> Result<ExitStatus, PlatformError> {
        self.input.take();
        let forced = self.child.kill().is_ok();
        let status = self
            .wait_until(Instant::now() + CLEANUP)
            .ok()
            .flatten()
            .ok_or_else(|| {
                PlatformError::CleanupUnconfirmed("fixture child did not close".into())
            })?;
        self.record_exit(status, forced);
        Ok(status)
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
