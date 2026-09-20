//! Fresh, account-authoritative shell environments. Never import Lens's launch environment.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

// Darwin's confstr selector: libc 0.2.189 src/unix/bsd/apple/mod.rs,
// _CS_DARWIN_USER_TEMP_DIR = 65537 (also macOS SDK unistd.h).
// Numeric definition keeps the shared Unix module type-checked on Linux;
// account() rejects other platforms before ever calling confstr with this selector.
const DARWIN_USER_TEMP_DIRECTORY: libc::c_int = 65537;
const CAPTURE_MODE: &str = "--lens-internal-capture-environment";
const MAX_PAYLOAD: usize = 1024 * 1024;
const RESOLUTION_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentPurpose {
    Session,
    Authentication,
    History,
    AccountStatus,
    Validation,
    Installation,
    Logout,
}

#[derive(Clone)]
pub struct ResolvedEnvironment {
    pub values: BTreeMap<OsString, OsString>,
    pub cwd: PathBuf,
    pub generation: Uuid,
    pub cwd_device: u64,
    pub cwd_inode: u64,
    pub purpose: EnvironmentPurpose,
}
impl ResolvedEnvironment {
    pub fn validate_directory(&self) -> Result<(), EnvironmentError> {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(&self.cwd).map_err(|_| EnvironmentError::Directory)?;
        if metadata.is_dir()
            && (metadata.dev(), metadata.ino()) == (self.cwd_device, self.cwd_inode)
        {
            Ok(())
        } else {
            Err(EnvironmentError::Directory)
        }
    }
}
/// Bind launch to an opened directory, so path replacement after validation
/// cannot apply one directory's environment to another directory.
pub fn bind_directory(
    command: &mut std::process::Command,
    cwd: &Path,
    identity: (u64, u64),
) -> Result<(), EnvironmentError> {
    use std::os::{
        fd::AsRawFd,
        unix::{fs::MetadataExt, process::CommandExt},
    };
    let directory = std::fs::File::open(cwd).map_err(|_| EnvironmentError::Directory)?;
    let metadata = directory
        .metadata()
        .map_err(|_| EnvironmentError::Directory)?;
    if !metadata.is_dir() || (metadata.dev(), metadata.ino()) != identity {
        return Err(EnvironmentError::Directory);
    }
    // SAFETY: the owned File keeps the descriptor alive through spawn/exec.
    // fchdir is async-signal-safe; no allocation or locking occurs on success.
    unsafe {
        command.pre_exec(move || {
            if libc::fchdir(directory.as_raw_fd()) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    Ok(())
}

impl fmt::Debug for ResolvedEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedEnvironment")
            .field("generation", &self.generation)
            .field("purpose", &self.purpose)
            .field("variable_count", &self.values.len())
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum EnvironmentError {
    #[error("Environment resolution is unsupported on this platform")]
    UnsupportedPlatform,

    #[error("Unable to obtain the operating-system account environment")]
    Account,

    #[error("The configured account shell is unsupported")]
    UnsupportedShell,
    #[error("The working directory is invalid or changed during environment resolution")]
    Directory,

    #[error("Unable to start the environment resolver")]
    Spawn,

    #[error("The shell environment resolver failed")]
    Activation,

    #[error("Environment resolution timed out")]
    Timeout,

    #[error("The environment capture protocol failed")]
    Protocol,

    #[error("The resolved environment exceeds the supported size")]
    Oversized,
}

#[derive(Serialize, Deserialize)]
struct Capture {
    version: u8,
    cwd: Vec<u8>,
    values: Vec<(Vec<u8>, Vec<u8>)>,
}

fn decode(capture: Capture) -> Result<(PathBuf, BTreeMap<OsString, OsString>), EnvironmentError> {
    use std::os::unix::ffi::OsStringExt;
    if capture.version != 1 || capture.cwd.contains(&0) {
        return Err(EnvironmentError::Protocol);
    }
    let cwd = PathBuf::from(OsString::from_vec(capture.cwd));
    if !cwd.is_absolute() {
        return Err(EnvironmentError::Protocol);
    }
    let mut values = BTreeMap::new();
    for (key, value) in capture.values {
        if key.is_empty()
            || key.contains(&0)
            || key.contains(&b'=')
            || value.contains(&0)
            || values
                .insert(OsString::from_vec(key), OsString::from_vec(value))
                .is_some()
        {
            return Err(EnvironmentError::Protocol);
        }
    }
    Ok((cwd, values))
}

/// Dispatch before constructing Tauri or touching standard ACP streams.
pub fn dispatch_capture_mode() -> Option<i32> {
    let mut args = std::env::args_os();
    args.next();
    if args.next().as_deref() != Some(OsStr::new(CAPTURE_MODE)) {
        return None;
    }
    let endpoint = args.next();
    if endpoint.is_none() || args.next().is_some() {
        return Some(2);
    }

    use std::{
        io::Write,
        os::unix::{ffi::OsStrExt, net::UnixStream},
    };
    let result = (|| -> Result<(), ()> {
        let cwd = std::env::current_dir().map_err(|_| ())?;
        let capture = Capture {
            version: 1,
            cwd: cwd.as_os_str().as_bytes().to_vec(),
            values: std::env::vars_os()
                .map(|(k, v)| (k.as_bytes().to_vec(), v.as_bytes().to_vec()))
                .collect(),
        };
        let payload = serde_json::to_vec(&capture).map_err(|_| ())?;
        if payload.len() > MAX_PAYLOAD {
            return Err(());
        }
        let mut socket = UnixStream::connect(endpoint.unwrap()).map_err(|_| ())?;
        socket
            .set_write_timeout(Some(RESOLUTION_TIMEOUT))
            .map_err(|_| ())?;
        socket
            .write_all(&(payload.len() as u32).to_be_bytes())
            .map_err(|_| ())?;
        socket.write_all(&payload).map_err(|_| ())?;
        Ok(())
    })();
    Some(if result.is_ok() { 0 } else { 2 })
}

struct Account {
    home: PathBuf,
    shell: PathBuf,
    name: OsString,
    temp: OsString,
}

fn account() -> Result<Account, EnvironmentError> {
    if !cfg!(target_os = "macos") {
        return Err(EnvironmentError::UnsupportedPlatform);
    }
    use std::{ffi::CStr, os::unix::ffi::OsStringExt};
    // getpwuid_r returns pointers into the retained scratch buffer; copy before dropping it.
    let mut buffer = vec![0u8; 65536];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return Err(EnvironmentError::Account);
    }
    let entry = unsafe { entry.assume_init() };
    let copy = |ptr: *const libc::c_char| -> Result<OsString, EnvironmentError> {
        if ptr.is_null() {
            return Err(EnvironmentError::Account);
        }
        Ok(OsString::from_vec(
            unsafe { CStr::from_ptr(ptr) }.to_bytes().to_vec(),
        ))
    };
    let home = PathBuf::from(copy(entry.pw_dir)?);
    let shell = PathBuf::from(copy(entry.pw_shell)?);
    if !home.is_absolute() || !shell.is_absolute() {
        return Err(EnvironmentError::Account);
    }
    let mut temp = vec![0u8; 4096];
    let len = unsafe {
        libc::confstr(
            DARWIN_USER_TEMP_DIRECTORY,
            temp.as_mut_ptr().cast(),
            temp.len(),
        )
    };
    if len == 0 || len > temp.len() {
        return Err(EnvironmentError::Account);
    }
    temp.truncate(len - 1);
    Ok(Account {
        home,
        shell,
        name: copy(entry.pw_name)?,
        temp: OsString::from_vec(temp),
    })
}

fn seed(account: &Account) -> BTreeMap<OsString, OsString> {
    // UTF-8 is an explicit acquisition policy, not an ambient terminal locale import.
    [
        ("HOME", account.home.as_os_str().to_owned()),
        ("USER", account.name.clone()),
        ("LOGNAME", account.name.clone()),
        ("SHELL", account.shell.as_os_str().to_owned()),
        ("TMPDIR", account.temp.clone()),
        ("PATH", OsString::from("/usr/bin:/bin:/usr/sbin:/sbin")),
        ("LANG", OsString::from("en_US.UTF-8")),
        ("LC_CTYPE", OsString::from("en_US.UTF-8")),
        ("TERM", OsString::from("dumb")),
    ]
    .into_iter()
    .map(|(k, v)| (OsString::from(k), v))
    .collect()
}

struct ShellGroup {
    child: Option<tokio::process::Child>,
    group: i32,
}

impl Drop for ShellGroup {
    fn drop(&mut self) {
        // The child owns a fresh group. This also cancels startup-script descendants.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            // Reap explicitly after cancellation while the runtime remains available.
            // On runtime teardown Child's Tokio orphan reaper is the final fallback.
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

/// Account home never comes from the process environment.
pub fn user_home() -> Result<PathBuf, EnvironmentError> {
    Ok(account()?.home)
}
/// Managed verification baseline, independent of user/project startup and caller PATH.
pub fn managed_seed() -> Result<BTreeMap<OsString, OsString>, EnvironmentError> {
    Ok(seed(&account()?))
}

/// Explicit opt-in tests may delegate self-exec modes to a built Lens binary.
#[cfg(test)]
pub(crate) fn test_helper_executable() -> Option<PathBuf> {
    std::env::var_os("LENS_TEST_AGENT_HELPER_EXECUTABLE").map(PathBuf::from)
}

/// Fresh acquisition; cancellation drops and terminates the owned shell group.
pub async fn resolve(
    cwd: &Path,
    purpose: EnvironmentPurpose,
) -> Result<ResolvedEnvironment, EnvironmentError> {
    let account = account()?;
    let helper = std::env::current_exe().map_err(|_| EnvironmentError::Spawn)?;
    #[cfg(test)]
    let helper = test_helper_executable().unwrap_or(helper);
    resolve_shell(cwd, purpose, &account.shell, &seed(&account), &helper).await
}

async fn resolve_shell(
    cwd: &Path,
    purpose: EnvironmentPurpose,
    shell: &Path,
    baseline: &BTreeMap<OsString, OsString>,
    helper: &Path,
) -> Result<ResolvedEnvironment, EnvironmentError> {
    use std::{
        os::unix::{fs::MetadataExt, process::CommandExt},
        process::Stdio,
    };
    use tokio::io::AsyncReadExt;
    if !cwd.is_absolute() {
        return Err(EnvironmentError::Directory);
    }
    let original = std::fs::metadata(cwd).map_err(|_| EnvironmentError::Directory)?;
    if !original.is_dir() {
        return Err(EnvironmentError::Directory);
    }
    // Acquire the initial prompt environment once, without a PTY or prompt rendering.
    // zsh runs chpwd on cd, then precmd and its registered hooks before a prompt.
    // bash's PROMPT_COMMAND can be either a scalar or an indexed array.
    let program = match shell.file_name().and_then(OsStr::to_str) {
        Some("zsh") => {
            r#"builtin cd -- "$1" || exit 70
() { local hook; if (( $+functions[precmd] )); then precmd; fi; for hook in "${precmd_functions[@]}"; do "$hook"; done; }
exec "$2" --lens-internal-capture-environment "$3""#
        }
        Some("bash") => {
            r#"builtin cd -- "$1" || exit 70
__lens_capture_prompt_hooks() { local hook; for hook in "${PROMPT_COMMAND[@]}"; do builtin eval -- "$hook"; done; }
__lens_capture_prompt_hooks
unset -f __lens_capture_prompt_hooks
exec "$2" --lens-internal-capture-environment "$3""#
        }
        _ => return Err(EnvironmentError::UnsupportedShell),
    };
    // Short OS-owned temporary root avoids both ambient TMPDIR and Unix socket path limits.
    let directory = tempfile::Builder::new()
        .prefix("lens-env-")
        .tempdir_in("/tmp")
        .map_err(|_| EnvironmentError::Spawn)?;
    let endpoint = directory.path().join("capture");
    let listener =
        tokio::net::UnixListener::bind(&endpoint).map_err(|_| EnvironmentError::Spawn)?;
    let mut command = tokio::process::Command::new(shell);
    command
        .args(["-l", "-i", "-c", program, "lens-environment"])
        .arg(cwd)
        .arg(helper)
        .arg(&endpoint)
        .env_clear()
        .envs(baseline)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    let process = command.spawn().map_err(|_| EnvironmentError::Spawn)?;
    let group = process.id().ok_or(EnvironmentError::Spawn)? as i32;
    let mut child = ShellGroup {
        child: Some(process),
        group,
    };
    let operation = async {
        let capture = async {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|_| EnvironmentError::Protocol)?;
            let len = stream
                .read_u32()
                .await
                .map_err(|_| EnvironmentError::Protocol)? as usize;
            if len > MAX_PAYLOAD {
                return Err(EnvironmentError::Oversized);
            }
            let mut payload = vec![0; len];
            stream
                .read_exact(&mut payload)
                .await
                .map_err(|_| EnvironmentError::Protocol)?;
            serde_json::from_slice::<Capture>(&payload).map_err(|_| EnvironmentError::Protocol)
        };
        let status = async {
            child
                .child
                .as_mut()
                .expect("owned child")
                .wait()
                .await
                .map_err(|_| EnvironmentError::Activation)
                .and_then(|status| {
                    if status.success() {
                        Ok(())
                    } else {
                        Err(EnvironmentError::Activation)
                    }
                })
        };
        let (capture, ()) = tokio::try_join!(capture, status)?;
        let (observed, values) = decode(capture)?;
        let current = std::fs::metadata(cwd).map_err(|_| EnvironmentError::Directory)?;
        let physical = std::fs::metadata(&observed).map_err(|_| EnvironmentError::Directory)?;
        if (original.dev(), original.ino()) != (current.dev(), current.ino())
            || (original.dev(), original.ino()) != (physical.dev(), physical.ino())
        {
            return Err(EnvironmentError::Directory);
        }
        Ok(ResolvedEnvironment {
            values,
            cwd: cwd.to_path_buf(),
            generation: Uuid::new_v4(),
            cwd_device: original.dev(),
            cwd_inode: original.ino(),
            purpose,
        })
    };
    let result = tokio::time::timeout(RESOLUTION_TIMEOUT, operation).await;
    match result {
        Ok(value) => value,
        Err(_) => {
            if let Some(id) = child.child.as_ref().expect("owned child").id() {
                unsafe {
                    libc::kill(-(id as i32), libc::SIGKILL);
                }
            }
            let _ = child.child.as_mut().expect("owned child").kill().await;
            Err(EnvironmentError::Timeout)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_preserves_unset_empty_and_non_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let (_, values) = decode(Capture {
            version: 1,
            cwd: b"/tmp".to_vec(),
            values: vec![(b"EMPTY".to_vec(), vec![]), (b"RAW".to_vec(), vec![255])],
        })
        .unwrap();
        assert_eq!(values.get(OsStr::new("EMPTY")).unwrap(), "");
        assert!(!values.contains_key(OsStr::new("ABSENT")));
        assert_eq!(values.get(OsStr::new("RAW")).unwrap().as_bytes(), &[255]);
    }
    #[test]
    fn rejects_ambiguous_or_invalid_exports() {
        for values in [
            vec![(b"A".to_vec(), vec![]), (b"A".to_vec(), vec![])],
            vec![(b"A=B".to_vec(), vec![])],
            vec![(b"A".to_vec(), vec![0])],
        ] {
            assert_eq!(
                decode(Capture {
                    version: 1,
                    cwd: b"/tmp".to_vec(),
                    values
                })
                .unwrap_err(),
                EnvironmentError::Protocol
            );
        }
    }
    #[test]
    fn debug_redacts_names_values_and_paths() {
        let value = ResolvedEnvironment {
            values: [("SECRET_NAME".into(), "SECRET_VALUE".into())].into(),
            cwd: "/SECRET_PATH".into(),
            generation: Uuid::nil(),
            cwd_device: 0,
            cwd_inode: 0,
            purpose: EnvironmentPurpose::Session,
        };
        let debug = format!("{value:?}");
        assert!(!debug.contains("SECRET"));
    }

    #[tokio::test]
    async fn shell_acquisition_is_fresh_cwd_sensitive_and_keeps_logical_pwd() {
        if !cfg!(target_os = "macos") {
            return;
        }
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let a = root.path().join("project a");
        let b = root.path().join("project b");
        for directory in [&home, &a, &b] {
            std::fs::create_dir(directory).unwrap();
        }
        let alias = root.path().join("logical a");
        symlink(&a, &alias).unwrap();
        std::fs::write(
            home.join(".zshrc"),
            r#"
export SHARED=baseline
function precmd() { export PRECMD_DIRECT=present; }
function fixture_precmd() { export PRECMD_REGISTERED=present; }
precmd_functions=(fixture_precmd)
function chpwd() {
  unset ONLY_A ONLY_B
  case "$PWD" in
    *'project a'|*'logical a') export ONLY_A=present; export PATH=/fixture/a:/usr/bin:/bin ;;
    *'project b') export ONLY_B=present; export PATH=/fixture/b:/usr/bin:/bin ;;
  esac
}
"#,
        )
        .unwrap();
        let helper = root.path().join("capture");
        std::fs::write(&helper, r#"#!/usr/bin/python3
import json, os, socket, struct, sys
payload=json.dumps({'version':1,'cwd':list(os.getcwdb()),'values':[(list(k),list(v)) for k,v in os.environb.items()]}).encode()
s=socket.socket(socket.AF_UNIX)
s.connect(sys.argv[2])
s.sendall(struct.pack('>I',len(payload))+payload)
"#).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let baseline = [
            (OsString::from("HOME"), home.into_os_string()),
            ("PATH".into(), "/usr/bin:/bin".into()),
        ]
        .into();
        let first = resolve_shell(
            &alias,
            EnvironmentPurpose::Session,
            Path::new("/bin/zsh"),
            &baseline,
            &helper,
        )
        .await
        .unwrap();
        assert_eq!(first.values.get(OsStr::new("ONLY_A")).unwrap(), "present");
        assert_eq!(
            first.values.get(OsStr::new("PWD")).unwrap(),
            alias.as_os_str()
        );
        let second = resolve_shell(
            &b,
            EnvironmentPurpose::Session,
            Path::new("/bin/zsh"),
            &baseline,
            &helper,
        )
        .await
        .unwrap();
        assert!(!second.values.contains_key(OsStr::new("ONLY_A")));
        assert_eq!(second.values.get(OsStr::new("ONLY_B")).unwrap(), "present");
        assert_eq!(
            second.values.get(OsStr::new("PATH")).unwrap(),
            "/fixture/b:/usr/bin:/bin"
        );
        assert_ne!(first.generation, second.generation);
        assert_eq!(
            second.values.get(OsStr::new("PRECMD_DIRECT")).unwrap(),
            "present"
        );
        assert_eq!(
            second.values.get(OsStr::new("PRECMD_REGISTERED")).unwrap(),
            "present"
        );
        let home = PathBuf::from(baseline.get(OsStr::new("HOME")).unwrap());
        std::fs::write(
            home.join(".bash_profile"),
            r#"export START_CWD="$PWD"
export STALE=present
PROMPT_COMMAND=('unset STALE; export PROMPT_FIRST=present' 'export PATH=/fixture/bash:/usr/bin:/bin')
"#,
        )
        .unwrap();
        let bash = resolve_shell(
            &b,
            EnvironmentPurpose::Session,
            Path::new("/bin/bash"),
            &baseline,
            &helper,
        )
        .await
        .unwrap();
        assert_eq!(
            bash.values.get(OsStr::new("START_CWD")).unwrap(),
            std::fs::canonicalize(&b).unwrap().as_os_str()
        );
        assert!(!bash.values.contains_key(OsStr::new("STALE")));
        assert_eq!(
            bash.values.get(OsStr::new("PROMPT_FIRST")).unwrap(),
            "present"
        );
        assert_eq!(
            bash.values.get(OsStr::new("PATH")).unwrap(),
            "/fixture/bash:/usr/bin:/bin"
        );
        std::fs::write(
            home.join(".bash_profile"),
            "export STALE=present\nPROMPT_COMMAND='unset STALE; export SCALAR_PROMPT=present'\n",
        )
        .unwrap();
        let scalar = resolve_shell(
            &b,
            EnvironmentPurpose::Session,
            Path::new("/bin/bash"),
            &baseline,
            &helper,
        )
        .await
        .unwrap();
        assert!(!scalar.values.contains_key(OsStr::new("STALE")));
        assert_eq!(
            scalar.values.get(OsStr::new("SCALAR_PROMPT")).unwrap(),
            "present"
        );
    }

    #[tokio::test]
    async fn failed_activation_has_no_secret_diagnostics_or_fallback() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(".zshenv"),
            "echo SECRET_SENTINEL >&2\nexit 23\n",
        )
        .unwrap();
        let baseline = [(OsString::from("HOME"), root.path().as_os_str().to_owned())].into();
        let error = resolve_shell(
            root.path(),
            EnvironmentPurpose::Session,
            Path::new("/bin/zsh"),
            &baseline,
            Path::new("/does/not/exist"),
        )
        .await
        .unwrap_err();
        assert_eq!(error, EnvironmentError::Activation);
        assert!(!format!("{error:?}: {error}").contains("SECRET_SENTINEL"));
        let error = resolve_shell(
            root.path(),
            EnvironmentPurpose::Session,
            Path::new("/bin/fish"),
            &baseline,
            Path::new("/does/not/exist"),
        )
        .await
        .unwrap_err();
        assert_eq!(error, EnvironmentError::UnsupportedShell);
    }

    #[tokio::test]
    async fn cancelled_resolution_kills_and_reaps_its_shell() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        std::fs::write(
            root.path().join(".zshenv"),
            "echo $$ > \"$PID_FILE\"\n/bin/sleep 60\n",
        )
        .unwrap();
        let baseline = [
            (OsString::from("HOME"), root.path().as_os_str().to_owned()),
            ("PID_FILE".into(), pid_file.as_os_str().to_owned()),
        ]
        .into();
        let cwd = root.path().to_owned();
        let task = tokio::spawn(async move {
            resolve_shell(
                &cwd,
                EnvironmentPurpose::Session,
                Path::new("/bin/zsh"),
                &baseline,
                Path::new("/does/not/exist"),
            )
            .await
        });
        let pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(value) = std::fs::read_to_string(&pid_file) {
                    if let Ok(pid) = value.trim().parse::<i32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), async {
            while unsafe { libc::kill(pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled resolver must terminate and reap its child");
    }

    #[test]
    fn snapshot_rejects_replaced_directory() {
        use std::os::unix::fs::MetadataExt;
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("cwd");
        std::fs::create_dir(&cwd).unwrap();
        let original = std::fs::metadata(&cwd).unwrap();
        let snapshot = ResolvedEnvironment {
            values: BTreeMap::new(),
            cwd: cwd.clone(),
            generation: Uuid::nil(),
            purpose: EnvironmentPurpose::Session,
            cwd_device: original.dev(),
            cwd_inode: original.ino(),
        };
        assert!(snapshot.validate_directory().is_ok());
        std::fs::rename(&cwd, root.path().join("old")).unwrap();
        std::fs::create_dir(&cwd).unwrap();
        assert_eq!(
            snapshot.validate_directory(),
            Err(EnvironmentError::Directory)
        );
    }

    #[tokio::test]
    async fn resolution_deadline_terminates_and_reaps_the_shell() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        std::fs::write(
            root.path().join(".zshenv"),
            "echo $$ > \"$PID_FILE\"\n/bin/sleep 60\n",
        )
        .unwrap();
        let baseline = [
            (OsString::from("HOME"), root.path().as_os_str().to_owned()),
            ("PID_FILE".into(), pid_file.as_os_str().to_owned()),
        ]
        .into();
        let result = resolve_shell(
            root.path(),
            EnvironmentPurpose::Session,
            Path::new("/bin/zsh"),
            &baseline,
            Path::new("/does/not/exist"),
        )
        .await;
        assert_eq!(result.unwrap_err(), EnvironmentError::Timeout);
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
