//! User-owned executable admission. Paths are data, never shell commands.
use crate::{agent_runtime::ResolvedAgentRuntime, model::AgentKind};
use std::path::Path;

pub(crate) fn validate_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err("Choose an absolute path to the ACP executable.".into());
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| "The ACP executable could not be found or accessed.".to_string())?;
    if !metadata.is_file() {
        return Err("The ACP executable path must name a file.".into());
    }
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("The selected ACP executable is not executable.".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_profile(profile: &crate::model::ExternalAgentProfile) -> Result<(), String> {
    profile.validate()?;
    if profile.command.is_absolute() {
        validate_path(&profile.command)
    } else {
        Ok(())
    }
}

#[derive(serde::Deserialize)]
pub(crate) struct ExternalAgentDraft {
    pub id: uuid::Uuid,
    pub name: String,
    pub command: String,
    pub arguments: String,
}
impl ExternalAgentDraft {
    pub fn parse(self) -> Result<crate::model::ExternalAgentProfile, String> {
        if self.command.len() + self.arguments.len() > 17408
            || self.command.contains(['\0', '\r', '\n'])
            || self.arguments.contains(['\0', '\r', '\n'])
        {
            return Err(
                "Executable and arguments must each be one line, contain no NUL, and use at most 17408 bytes combined.".into(),
            );
        }
        let mut quote = None;
        let mut escaped = false;
        for c in self.arguments.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' && quote != Some('\'') {
                escaped = true;
                continue;
            }
            if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
                continue;
            }
            if c == '\'' || c == '"' {
                quote = Some(c);
                continue;
            }
            if "|;&<>$`()~#".contains(c) {
                return Err(
                    "Shell operators and expansions are not supported. Quote literal arguments."
                        .into(),
                );
            }
        }
        let args = shlex::split(&self.arguments)
            .ok_or("Arguments contain an unfinished quote or escape.")?;
        let profile = crate::model::ExternalAgentProfile {
            id: self.id,
            name: self.name,
            command: self.command.into(),
            args,
        };
        profile.validate()?;
        Ok(profile)
    }
}

pub(crate) fn resolve_command(
    command: &Path,
    environment: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    cwd: &Path,
) -> Result<std::path::PathBuf, String> {
    if command.is_absolute() {
        validate_path(command)?;
        return Ok(command.to_path_buf());
    }
    let path = environment
        .get(std::ffi::OsStr::new("PATH"))
        .ok_or("The resolved user environment does not provide PATH.")?;
    for directory in std::env::split_paths(path) {
        let directory = if directory.is_absolute() {
            directory
        } else {
            cwd.join(directory)
        };
        let candidate = directory.join(command);
        if validate_path(&candidate).is_ok() {
            return Ok(candidate);
        }
    }
    Err("The Agent executable was not found in your resolved PATH. Install it or provide an absolute path.".into())
}

pub(crate) async fn resolve(
    profile: crate::model::ExternalAgentProfile,
    cwd: &Path,
) -> Result<ResolvedAgentRuntime, String> {
    validate_profile(&profile)?;
    let environment = if profile.command.is_absolute() {
        std::collections::BTreeMap::new()
    } else {
        crate::agent_environment::resolve(
            cwd,
            crate::agent_environment::EnvironmentPurpose::Validation,
        )
        .await
        .map_err(|error| error.to_string())?
        .values
    };
    admit_runtime(profile, &environment, cwd)
}

fn admit_runtime(
    profile: crate::model::ExternalAgentProfile,
    environment: &std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString>,
    cwd: &Path,
) -> Result<ResolvedAgentRuntime, String> {
    resolve_command(&profile.command, environment, cwd)?;
    // This is installation readiness only. Keep the original command so each
    // physical connection still acquires and uses its own fresh environment.
    Ok(ResolvedAgentRuntime {
        kind: AgentKind::External(profile.id),
        adapter_name: "external-acp",
        adapter_version: "unknown".into(),
        command: profile.command,
        args: profile.args,
        installation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn literal_profiles_do_not_execute_version_commands() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let command = dir.join("custom \u{65e5} $(literal)");
        std::fs::write(&command, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = crate::model::ExternalAgentProfile {
            id: uuid::Uuid::new_v4(),
            name: "Custom".into(),
            command: command.clone(),
            args: vec!["--stdio".into(), "space argument".into()],
        };
        let runtime = resolve(profile.clone(), &dir).await.unwrap();
        assert_eq!(runtime.command, command);
        assert_eq!(runtime.args, profile.args);
        assert!(runtime.installation.is_none());
        let mut invalid = profile;
        invalid.args = vec!["bad\0arg".into()];
        assert!(validate_profile(&invalid).is_err());
        invalid.args = vec!["x".repeat(16385)];
        assert!(validate_profile(&invalid).is_err());
        assert!(validate_path(Path::new("relative")).is_err());
        assert!(validate_path(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn draft_preserves_literal_executable_and_parses_only_arguments() {
        let command = "/Applications/My Agent's CLI/agent $(literal)";
        let arguments = r#"acp '' '$HOME' "$HOME" escaped\|pipe"#;
        let profile = ExternalAgentDraft {
            id: uuid::Uuid::from_u128(1),
            name: "Custom".into(),
            command: command.into(),
            arguments: arguments.into(),
        }
        .parse()
        .unwrap();
        assert_eq!(profile.command, Path::new(command));
        assert_eq!(profile.args, ["acp", "", "$HOME", "$HOME", "escaped|pipe"]);
        let roundtrip = ExternalAgentDraft {
            id: profile.id,
            name: profile.name.clone(),
            command: profile.command.to_str().unwrap().into(),
            arguments: shlex::try_join(profile.args.iter().map(String::as_str)).unwrap(),
        }
        .parse()
        .unwrap();
        assert_eq!(roundtrip, profile);
        let no_arguments = ExternalAgentDraft {
            id: profile.id,
            name: profile.name,
            command: "goose".into(),
            arguments: String::new(),
        }
        .parse()
        .unwrap();
        assert!(no_arguments.args.is_empty());
    }

    #[test]
    fn draft_rejects_invalid_executables_and_argument_syntax() {
        let parse = |command: &str, arguments: &str| {
            ExternalAgentDraft {
                id: uuid::Uuid::from_u128(1),
                name: "Custom".into(),
                command: command.into(),
                arguments: arguments.into(),
            }
            .parse()
        };
        for command in ["", "./goose", "~/goose", "goose\0", "goose\r", "goose\n"] {
            assert!(parse(command, "").is_err(), "{command:?}");
        }
        for arguments in [
            "| other",
            "$HOME",
            "`id`",
            "'unfinished",
            "unfinished\\",
            "acp\0",
            "acp\r",
            "acp\n",
            "# comment",
        ] {
            assert!(parse("goose", arguments).is_err(), "{arguments:?}");
        }
        assert!(parse("goose", &"x ".repeat(65)).is_err());
        assert!(parse("goose", &"x".repeat(16385)).is_err());
        assert!(parse(&"x".repeat(17409), "").is_err());
        assert!(parse(&"x".repeat(1025), &"x".repeat(16384)).is_err());
    }

    #[test]
    fn executable_lookup_uses_only_captured_path_and_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let command = bin.join("custom-agent");
        std::fs::write(&command, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        for path in [bin.as_os_str().to_owned(), std::ffi::OsString::from("bin")] {
            let env = std::collections::BTreeMap::from([("PATH".into(), path)]);
            assert_eq!(
                resolve_command(Path::new("custom-agent"), &env, root.path()).unwrap(),
                command
            );
        }
        let env = std::collections::BTreeMap::from([("PATH".into(), std::ffi::OsString::new())]);
        assert_eq!(
            resolve_command(Path::new("custom-agent"), &env, &bin).unwrap(),
            command
        );
        assert!(resolve_command(
            Path::new("custom-agent"),
            &std::collections::BTreeMap::new(),
            root.path()
        )
        .is_err());
    }
    #[test]
    fn runtime_admission_requires_executable_in_resolved_path() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let command = root.path().join("fixture-agent");
        let profile = crate::model::ExternalAgentProfile {
            id: uuid::Uuid::new_v4(),
            name: "Fixture".into(),
            command: "fixture-agent".into(),
            args: vec!["acp".into()],
        };
        let environment =
            std::collections::BTreeMap::from([("PATH".into(), root.path().as_os_str().to_owned())]);
        assert!(admit_runtime(profile.clone(), &environment, root.path())
            .unwrap_err()
            .contains("not found"));
        std::fs::write(&command, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(admit_runtime(profile.clone(), &environment, root.path()).is_err());
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = admit_runtime(profile.clone(), &environment, root.path()).unwrap();
        // Preflight never executes the CLI and never freezes its PATH resolution.
        assert_eq!(runtime.command, profile.command);
        assert_eq!(runtime.args, profile.args);
    }
}
