//! Apply an immutable environment without replacing the ACP SDK's process owner.
//!
//! The self-exec helper receives an in-memory plan over a private one-shot socket.
//! Its stdin/stdout remain exclusively ACP; exec preserves the SDK-owned PID/group.
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    os::fd::AsRawFd,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{DirBuilderExt, MetadataExt},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use agent_client_protocol::{
    AcpAgent, AcpAgentConfig, Agent, Client, ConnectTo, DynConnectTo, Error, ErrorCode,
};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, net::UnixListener};

const HELPER_MODE: &str = "--lens-internal-agent-exec";
const PROTOCOL_VERSION: u32 = 1;
const MAX_PLAN_BYTES: usize = 1024 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Intentionally does not implement Debug: arguments and environment may be secret.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchPlan {
    version: u32,
    command: Vec<u8>,
    args: Vec<Vec<u8>>,
    cwd: Vec<u8>,
    cwd_device: u64,
    cwd_inode: u64,
    environment: Vec<(Vec<u8>, Vec<u8>)>,
}

impl LaunchPlan {
    fn new(
        command: PathBuf,
        args: Vec<OsString>,
        cwd: PathBuf,
        environment: BTreeMap<OsString, OsString>,
    ) -> Result<Self, &'static str> {
        let metadata = std::fs::metadata(&cwd).map_err(|_| "agent launch directory unavailable")?;
        if !metadata.is_dir() {
            return Err("agent launch directory invalid");
        }
        let plan = Self {
            version: PROTOCOL_VERSION,
            command: command.into_os_string().into_vec(),
            args: args.into_iter().map(OsString::into_vec).collect(),
            cwd: cwd.into_os_string().into_vec(),
            cwd_device: metadata.dev(),
            cwd_inode: metadata.ino(),
            environment: environment
                .into_iter()
                .map(|(k, v)| (k.into_vec(), v.into_vec()))
                .collect(),
        };
        plan.validate()?;
        Ok(plan)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROTOCOL_VERSION {
            return Err("agent launch protocol invalid");
        }
        if !Path::new(OsStr::from_bytes(&self.command)).is_absolute()
            || !Path::new(OsStr::from_bytes(&self.cwd)).is_absolute()
        {
            return Err("agent launch path invalid");
        }
        if self.command.contains(&0)
            || self.cwd.contains(&0)
            || self.args.iter().any(|a| a.contains(&0))
            || self
                .environment
                .iter()
                .any(|(k, v)| k.is_empty() || k.contains(&b'=') || k.contains(&0) || v.contains(&0))
        {
            return Err("agent launch encoding invalid");
        }
        // ARG_MAX covers strings AND pointer tables. Stay below the platform limit;
        // the kernel remains authoritative if platform-specific overhead differs.
        let string_bytes = self.command.len()
            + 1
            + self.args.iter().map(|a| a.len() + 1).sum::<usize>()
            + self
                .environment
                .iter()
                .map(|(k, v)| k.len() + v.len() + 2)
                .sum::<usize>();
        let pointer_bytes =
            (self.args.len() + self.environment.len() + 3) * std::mem::size_of::<usize>();
        // SAFETY: sysconf has no pointer arguments or side effects on Rust memory.
        let arg_max = unsafe { libc::sysconf(libc::_SC_ARG_MAX) };
        if arg_max <= 0
            || string_bytes
                .saturating_add(pointer_bytes)
                .saturating_add(4096)
                > arg_max as usize
        {
            return Err("agent launch environment too large");
        }
        Ok(())
    }

    fn command(&self) -> Result<Command, &'static str> {
        self.validate()?;
        let cwd = Path::new(OsStr::from_bytes(&self.cwd));
        let mut command = Command::new(OsStr::from_bytes(&self.command));
        crate::agent_environment::bind_directory(
            &mut command,
            cwd,
            (self.cwd_device, self.cwd_inode),
        )
        .map_err(|_| "agent launch directory changed")?;
        command
            .args(self.args.iter().map(|a| OsStr::from_bytes(a)))
            .env_clear()
            .envs(
                self.environment
                    .iter()
                    .map(|(k, v)| (OsStr::from_bytes(k), OsStr::from_bytes(v))),
            );
        Ok(command)
    }
}

struct Endpoint {
    directory: PathBuf,
    path: PathBuf,
}

impl Endpoint {
    fn create() -> Result<(Self, UnixListener), &'static str> {
        // macOS's account temporary directory can exceed sockaddr_un.sun_path.
        // A random owner-only directory beneath the OS /tmp keeps the path short.
        let directory =
            PathBuf::from("/tmp").join(format!("lens-exec-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| "agent launch endpoint unavailable")?;
        let endpoint = Self {
            path: directory.join("s"),
            directory,
        };
        let listener =
            UnixListener::bind(&endpoint.path).map_err(|_| "agent launch endpoint unavailable")?;
        Ok((endpoint, listener))
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

/// Raw SDK errors can include the child's stderr. Never return them to diagnostics.
fn redact_transport_error(error: Error) -> Error {
    if error.code == ErrorCode::InternalError {
        redacted_error("agent transport failed")
    } else {
        error
    }
}

fn redacted_error(category: &'static str) -> Error {
    Error::internal_error().data(category)
}

struct LaunchTransport {
    directory_identity: Option<(u64, u64)>,
    #[cfg(test)]
    helper_executable: Option<PathBuf>,
    command: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    environment: BTreeMap<OsString, OsString>,
}

impl ConnectTo<Client> for LaunchTransport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), Error> {
        let plan = LaunchPlan::new(self.command, self.args, self.cwd, self.environment)
            .map_err(redacted_error)?;
        if let Some((device, inode)) = self.directory_identity {
            if (plan.cwd_device, plan.cwd_inode) != (device, inode) {
                return Err(redacted_error("agent launch directory changed"));
            }
        }
        let payload = serde_json::to_vec(&plan)
            .map_err(|_| redacted_error("agent launch protocol invalid"))?;
        if payload.len() > MAX_PLAN_BYTES {
            return Err(redacted_error("agent launch payload too large"));
        }
        let (endpoint, listener) = Endpoint::create().map_err(redacted_error)?;
        let executable = std::env::current_exe()
            .map_err(|_| redacted_error("agent launch executable unavailable"))?;
        #[cfg(test)]
        let executable = self.helper_executable.unwrap_or(executable);
        // The endpoint is ASCII and contains no environment values or credentials.
        let agent = AcpAgent::new(
            AcpAgentConfig::new(executable)
                .arg(HELPER_MODE)
                .arg(endpoint.path.to_string_lossy()),
        );
        let connection = ConnectTo::<Client>::connect_to(agent, client);
        tokio::pin!(connection);
        let broker = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|_| redacted_error("agent launch handshake failed"))?;
            stream
                .write_all(&(payload.len() as u32).to_be_bytes())
                .await
                .map_err(|_| redacted_error("agent launch handshake failed"))?;
            stream
                .write_all(&payload)
                .await
                .map_err(|_| redacted_error("agent launch handshake failed"))?;
            stream
                .shutdown()
                .await
                .map_err(|_| redacted_error("agent launch handshake failed"))?;
            Ok::<_, Error>(())
        });
        tokio::pin!(broker);
        tokio::select! {
            result = &mut connection => result.map_err(redact_transport_error),
            result = &mut broker => {
                result.map_err(|_| redacted_error("agent launch handshake timed out"))??;
                // Plan delivery is not ACP readiness; retain the SDK owner until
                // the real transport completes, including EOF/write drain.
                connection.await.map_err(redact_transport_error)
            }
        }
    }
}

/// A Terminal-owned command uses the same secret-free plan handoff, but has no
/// ACP SDK child ownership. Its endpoint owner must live until serve completes.
pub(crate) struct ExternalLaunch {
    pub executable: PathBuf,
    pub arguments: Vec<OsString>,
    endpoint: Endpoint,
    listener: UnixListener,
    payload: Vec<u8>,
}

impl ExternalLaunch {
    pub(crate) async fn serve(self) -> Result<(), &'static str> {
        let result = tokio::time::timeout(Duration::from_secs(60), async {
            let (mut stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| "authentication launch handshake failed")?;
            stream
                .write_all(&(self.payload.len() as u32).to_be_bytes())
                .await
                .map_err(|_| "authentication launch handshake failed")?;
            stream
                .write_all(&self.payload)
                .await
                .map_err(|_| "authentication launch handshake failed")?;
            stream
                .shutdown()
                .await
                .map_err(|_| "authentication launch handshake failed")?;
            Ok(())
        })
        .await
        .map_err(|_| "authentication launch handshake timed out")?;
        drop(self.endpoint);
        result
    }
}

/// Prepare before opening Terminal, then drive serve concurrently. Aborting
/// that future removes the endpoint. Only executable/arguments belong in the
/// .command script; the raw environment remains in this in-memory owner.
pub(crate) fn prepare_external_launch(
    command: PathBuf,
    args: Vec<OsString>,
    resolved: crate::agent_environment::ResolvedEnvironment,
) -> Result<ExternalLaunch, &'static str> {
    let directory_identity = (resolved.cwd_device, resolved.cwd_inode);
    let plan = LaunchPlan::new(command, args, resolved.cwd, resolved.values)?;
    if (plan.cwd_device, plan.cwd_inode) != directory_identity {
        return Err("authentication launch directory changed");
    }
    let payload =
        serde_json::to_vec(&plan).map_err(|_| "authentication launch protocol invalid")?;
    if payload.len() > MAX_PLAN_BYTES {
        return Err("authentication launch payload too large");
    }
    let (endpoint, listener) = Endpoint::create()?;
    let executable =
        std::env::current_exe().map_err(|_| "authentication launch executable unavailable")?;
    let arguments = vec![
        OsString::from(HELPER_MODE),
        endpoint.path.clone().into_os_string(),
    ];
    Ok(ExternalLaunch {
        executable,
        arguments,
        endpoint,
        listener,
        payload,
    })
}

pub(crate) fn prepare_working_command(
    command: &Path,
    resolved: &mut crate::agent_environment::ResolvedEnvironment,
    managed: bool,
) -> Result<PathBuf, String> {
    if managed {
        resolved.values.insert("NODE_OPTIONS".into(), "".into());
        resolved.values.insert("NODE_PATH".into(), "".into());
        Ok(command.to_path_buf())
    } else {
        crate::external_agent::resolve_command(command, &resolved.values, &resolved.cwd)
    }
}

struct WorkingTransport {
    command: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    purpose: crate::agent_environment::EnvironmentPurpose,
    managed: bool,
}

impl ConnectTo<Client> for WorkingTransport {
    async fn connect_to(self, client: impl ConnectTo<Agent>) -> Result<(), Error> {
        let mut resolved = crate::agent_environment::resolve(&self.cwd, self.purpose)
            .await
            .map_err(|error| Error::internal_error().data(error.to_string()))?;
        let command = prepare_working_command(&self.command, &mut resolved, self.managed)
            .map_err(|error| Error::internal_error().data(error))?;
        let environment = resolved.values;
        LaunchTransport {
            directory_identity: Some((resolved.cwd_device, resolved.cwd_inode)),
            #[cfg(test)]
            helper_executable: crate::agent_environment::test_helper_executable(),
            command,
            args: self.args,
            cwd: resolved.cwd,
            environment,
        }
        .connect_to(client)
        .await
    }
}

/// Resolution is owned by the connection future: dropping it cancels shell
/// acquisition before any provider is spawned, and no cross-session cache exists.
pub(crate) fn working_transport(
    command: PathBuf,
    args: Vec<OsString>,
    cwd: PathBuf,
    purpose: crate::agent_environment::EnvironmentPurpose,
    managed: bool,
) -> DynConnectTo<Client> {
    DynConnectTo::new(WorkingTransport {
        command,
        args,
        cwd,
        purpose,
        managed,
    })
}

fn receive_plan(endpoint: &Path) -> Result<LaunchPlan, &'static str> {
    let mut stream = UnixStream::connect(endpoint).map_err(|_| "agent launch handshake failed")?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut header = [0; 4];
    read_before_deadline(&mut stream, &mut header, deadline)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_PLAN_BYTES {
        return Err("agent launch payload too large");
    }
    let mut payload = vec![0; length];
    read_before_deadline(&mut stream, &mut payload, deadline)?;
    let plan: LaunchPlan =
        serde_json::from_slice(&payload).map_err(|_| "agent launch protocol invalid")?;
    plan.validate()?;
    Ok(plan)
}

fn read_before_deadline(
    stream: &mut UnixStream,
    mut buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), &'static str> {
    while !buffer.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("agent launch handshake timed out")?;
        if remaining.is_zero() {
            return Err("agent launch handshake timed out");
        }
        // SO_RCVTIMEO fails with EINVAL in the macOS helper on
        // supported runtime builds. poll uses a millisecond deadline and keeps
        // this control read independent of platform timeval representation.
        stream
            .set_nonblocking(true)
            .map_err(|_| "agent launch handshake failed")?;
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout_ms = remaining.as_millis().max(1).min(i32::MAX as u128) as i32;
        // SAFETY: descriptor points to one initialized pollfd for the duration
        // of the syscall, and its stream remains owned and open here.
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if ready == 0 {
            return Err("agent launch handshake timed out");
        }
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("agent launch handshake failed");
        }
        let length = match stream.read(buffer) {
            Ok(length) => length,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(_) => return Err("agent launch payload read failed"),
        };
        if length == 0 {
            return Err("agent launch payload incomplete");
        }
        buffer = &mut buffer[length..];
    }
    Ok(())
}

/// Invoke before Tauri/native initialization. Malformed internal invocations must
/// exit as a helper, rather than accidentally opening a second GUI instance.
pub fn early_helper_exit() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(HELPER_MODE)) {
        return None;
    }
    let result = match (args.next(), args.next()) {
        (Some(endpoint), None) => receive_plan(Path::new(&endpoint)).and_then(|plan| {
            let mut command = plan.command()?;
            // Never fork/setsid here: the SDK owns this PID and process group.
            let _error = command.exec();
            Err::<(), _>("agent launch exec failed")
        }),
        _ => Err("agent launch arguments invalid"),
    };
    if let Err(category) = result {
        let _ = writeln!(std::io::stderr(), "{category}");
    }
    Some(126)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(command: &str, args: &[&str]) -> LaunchPlan {
        LaunchPlan::new(
            PathBuf::from(command),
            args.iter().map(OsString::from).collect(),
            PathBuf::from("/tmp"),
            BTreeMap::from([
                (OsString::from("EMPTY"), OsString::new()),
                (OsString::from("PATH"), OsString::from("/project/bin")),
            ]),
        )
        .unwrap()
    }

    #[test]
    fn exact_environment_preserves_empty_and_removes_inherited_values() {
        let output = plan("/usr/bin/env", &[])
            .command()
            .unwrap()
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut lines = output
            .stdout
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| l.to_vec())
            .collect::<Vec<_>>();
        lines.sort();
        assert_eq!(
            lines,
            vec![b"EMPTY=".to_vec(), b"PATH=/project/bin".to_vec()]
        );
    }

    #[tokio::test]
    async fn command_keeps_resolved_directory_when_symlink_changes_before_spawn() {
        use std::os::unix::fs::symlink;
        let (endpoint, listener) = Endpoint::create().unwrap();
        drop(listener);
        let original = endpoint.directory.join("original");
        let replacement = endpoint.directory.join("replacement");
        let logical = endpoint.directory.join("logical");
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        symlink(&original, &logical).unwrap();
        let plan = LaunchPlan::new(
            PathBuf::from("/bin/pwd"),
            vec![],
            logical.clone(),
            BTreeMap::new(),
        )
        .unwrap();
        let mut command = plan.command().unwrap();
        std::fs::remove_file(&logical).unwrap();
        symlink(&replacement, &logical).unwrap();
        let output = command.output().unwrap();
        assert!(output.status.success());
        let observed = PathBuf::from(OsString::from_vec(
            output.stdout.strip_suffix(b"\n").unwrap().to_vec(),
        ));
        assert_eq!(
            observed.canonicalize().unwrap(),
            original.canonicalize().unwrap()
        );
        std::fs::remove_file(logical).unwrap();
        std::fs::remove_dir(original).unwrap();
        std::fs::remove_dir(replacement).unwrap();
    }

    #[test]
    fn private_plan_round_trip_preserves_os_bytes() {
        let mut plan = plan("/usr/bin/env", &[]);
        plan.environment.push((b"BYTES".to_vec(), vec![0xff, 0xfe]));
        let encoded = serde_json::to_vec(&plan).unwrap();
        let decoded: LaunchPlan = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.environment, plan.environment);
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn invalid_fields_and_exec_size_fail_before_execution() {
        let mut plan = plan("/usr/bin/env", &[]);
        plan.environment.push((b"INVALID=KEY".to_vec(), vec![]));
        assert_eq!(plan.validate(), Err("agent launch encoding invalid"));
        plan.environment.pop();
        plan.args.push(vec![b'x'; 4 * 1024 * 1024]);
        assert_eq!(plan.validate(), Err("agent launch environment too large"));
    }

    #[test]
    fn replaced_directory_identity_is_rejected() {
        let mut plan = plan("/usr/bin/env", &[]);
        plan.cwd_inode = plan.cwd_inode.wrapping_add(1);
        assert!(matches!(
            plan.command(),
            Err("agent launch directory changed")
        ));
    }

    #[tokio::test]
    async fn endpoint_is_private_short_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let (endpoint, listener) = Endpoint::create().unwrap();
        let directory = endpoint.directory.clone();
        let path = endpoint.path.clone();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(path.as_os_str().len() < 104);
        drop(listener);
        drop(endpoint);
        assert!(!path.exists());
        assert!(!directory.exists());
    }

    #[test]
    fn bounded_protocol_rejects_oversize_before_allocating_payload() {
        let directory = PathBuf::from("/tmp").join(format!(
            "lens-launch-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let endpoint = Endpoint {
            path: directory.join("s"),
            directory,
        };
        let listener = std::os::unix::net::UnixListener::bind(&endpoint.path).unwrap();
        let sender = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .write_all(&((MAX_PLAN_BYTES + 1) as u32).to_be_bytes())
                .unwrap();
        });
        assert!(matches!(
            receive_plan(&endpoint.path),
            Err("agent launch payload too large")
        ));
        sender.join().unwrap();
    }

    #[test]
    fn handshake_deadline_bounds_slow_peer() {
        let (mut receiver, _silent_peer) = UnixStream::pair().unwrap();
        let began = Instant::now();
        let result = read_before_deadline(
            &mut receiver,
            &mut [0; 1],
            began + Duration::from_millis(20),
        );
        assert!(result.is_err());
        assert!(began.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn exact_command_does_not_consume_first_protocol_bytes() {
        use std::process::Stdio;
        let mut child = plan("/bin/cat", &[])
            .command()
            .unwrap()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let frame = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n";
        child.stdin.take().unwrap().write_all(frame).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.stdout, frame);
        assert!(output.status.success());
    }

    fn resolved_for_test(
        values: BTreeMap<OsString, OsString>,
    ) -> crate::agent_environment::ResolvedEnvironment {
        let cwd = PathBuf::from("/tmp");
        let metadata = std::fs::metadata(&cwd).unwrap();
        crate::agent_environment::ResolvedEnvironment {
            values,
            cwd,
            cwd_device: metadata.dev(),
            cwd_inode: metadata.ino(),
            generation: uuid::Uuid::new_v4(),
            purpose: crate::agent_environment::EnvironmentPurpose::Authentication,
        }
    }

    #[tokio::test]
    async fn terminal_broker_rejects_resolution_from_another_directory_identity() {
        let mut resolved = resolved_for_test(BTreeMap::new());
        resolved.cwd_inode = resolved.cwd_inode.wrapping_add(1);
        assert!(matches!(
            prepare_external_launch(PathBuf::from("/usr/bin/env"), vec![], resolved),
            Err("authentication launch directory changed")
        ));
    }

    #[tokio::test]
    async fn terminal_broker_keeps_values_off_arguments_and_cleans_after_cancellation() {
        let launch = prepare_external_launch(
            PathBuf::from("/usr/bin/env"),
            vec![],
            resolved_for_test(BTreeMap::from([(
                OsString::from("SECRET"),
                OsString::from("sentinel-secret"),
            )])),
        )
        .unwrap();
        assert!(!format!("{:?}", launch.arguments).contains("sentinel-secret"));
        let path = PathBuf::from(&launch.arguments[1]);
        assert!(path.exists());
        let task = tokio::spawn(launch.serve());
        tokio::task::yield_now().await;
        task.abort();
        let _ = task.await;
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn terminal_broker_sends_one_complete_plan_and_removes_endpoint() {
        let launch = prepare_external_launch(
            PathBuf::from("/usr/bin/env"),
            vec![],
            resolved_for_test(BTreeMap::new()),
        )
        .unwrap();
        let path = PathBuf::from(&launch.arguments[1]);
        let receiver_path = path.clone();
        let receiver = tokio::task::spawn_blocking(move || receive_plan(&receiver_path));
        launch.serve().await.unwrap();
        let plan = receiver.await.unwrap().unwrap();
        assert_eq!(plan.command, b"/usr/bin/env");
        assert!(!path.exists());
    }

    /// Run after building the real desktop binary. This exercises SDK ownership
    /// across the actual self-exec boundary, not a mock process transport.
    #[tokio::test]
    #[ignore = "requires LENS_TEST_EXECUTABLE pointing to the built lens binary"]
    async fn actual_helper_cancellation_kills_sdk_process_group() {
        let helper = PathBuf::from(
            std::env::var_os("LENS_TEST_EXECUTABLE").expect("set LENS_TEST_EXECUTABLE"),
        );
        let (endpoint, listener) = Endpoint::create().unwrap();
        drop(listener);
        let pids = endpoint.directory.join("pids");
        let launch = LaunchTransport {
            directory_identity: None,
            helper_executable: Some(helper),
            command: PathBuf::from("/bin/sh"),
            args: vec![
                OsString::from("-c"),
                OsString::from("/bin/sleep 60 & echo $$ $! > \"$1\"; wait"),
                OsString::from("lens-test"),
                pids.clone().into_os_string(),
            ],
            cwd: PathBuf::from("/tmp"),
            environment: BTreeMap::new(),
        };
        let (_channel, future) = launch.into_channel_and_future();
        let connection = tokio::spawn(future);
        let processes: Vec<i32> = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(&pids) {
                    let ids = value
                        .split_whitespace()
                        .map(|p| p.parse().unwrap())
                        .collect::<Vec<i32>>();
                    if ids.len() == 2 {
                        break ids;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("helper did not exec fixture");
        connection.abort();
        let _ = connection.await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if processes.iter().all(|pid| {
                    // SAFETY: signal 0 observes a process; no pointers are used.
                    unsafe { libc::kill(*pid, 0) != 0 }
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SDK cancellation left a live group member");
        std::fs::remove_file(pids).unwrap();
    }

    #[tokio::test]
    #[ignore = "requires LENS_TEST_EXECUTABLE pointing to the built lens binary"]
    async fn actual_helper_stderr_failure_is_redacted() {
        let helper = PathBuf::from(
            std::env::var_os("LENS_TEST_EXECUTABLE").expect("set LENS_TEST_EXECUTABLE"),
        );
        let (endpoint, listener) = Endpoint::create().unwrap();
        drop(listener);
        let marker = endpoint.directory.join("started");
        let launch = LaunchTransport {
            directory_identity: None,
            helper_executable: Some(helper),
            command: PathBuf::from("/bin/sh"),
            args: vec![
                OsString::from("-c"),
                OsString::from(
                    "printf started > \"$1\"; /usr/bin/yes noise | /usr/bin/head -c 1048576 >&2; printf sentinel-secret >&2; exit 17",
                ),
                OsString::from("lens-test"),
                marker.clone().into_os_string(),
            ],
            cwd: PathBuf::from("/tmp"),
            environment: BTreeMap::new(),
        };
        let (_channel, future) = launch.into_channel_and_future();
        let error = tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "started");
        std::fs::remove_file(marker).unwrap();
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(!format!("{error:?}").contains("sentinel-secret"));
    }

    #[test]
    fn redaction_does_not_include_sdk_stderr() {
        let error = redact_transport_error(Error::internal_error().data("sentinel-secret"));
        assert!(!format!("{error:?}").contains("sentinel-secret"));
        assert_eq!(
            redact_transport_error(Error::auth_required()).code,
            ErrorCode::AuthRequired
        );
    }
    #[test]
    fn acp_and_terminal_auth_share_lookup_and_environment_policy() {
        let values = BTreeMap::from([
            ("PATH".into(), "/bin:/usr/bin".into()),
            ("NODE_OPTIONS".into(), "user-owned-value".into()),
            ("AUTH_OVERLAY".into(), "method-value".into()),
        ]);
        let mut external = resolved_for_test(values.clone());
        let command = prepare_working_command(Path::new("sh"), &mut external, false).unwrap();
        assert!(command.is_absolute());
        assert_eq!(external.values, values);
        assert_eq!(
            prepare_working_command(&command, &mut external, false).unwrap(),
            command
        );
        let mut managed = resolved_for_test(values);
        prepare_working_command(Path::new("/managed/node"), &mut managed, true).unwrap();
        assert_eq!(
            managed.values.get(OsStr::new("NODE_OPTIONS")),
            Some(&OsString::new())
        );
        assert_eq!(
            managed.values.get(OsStr::new("AUTH_OVERLAY")),
            Some(&OsString::from("method-value"))
        );
    }
}
