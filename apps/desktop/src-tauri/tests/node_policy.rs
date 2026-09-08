#[path = "../node_policy.rs"]
mod node_policy;

const CONFIG: &str = "[tools]\nnode = '24.20.0'";
const LOCK: &str = r#"
[[tools.node]]
version = "24.20.0"
backend = "core:node"
[tools.node."platforms.macos-arm64"]
url = "https://nodejs.org/dist/v24.20.0/node-v24.20.0-darwin-arm64.tar.gz"
checksum = "sha256:40e5607e5ecb3db9192723776da2d75d966260fc74a7a9e731c1bd67dda96bc8"
"#;

#[test]
fn repository_inputs_define_the_managed_artifact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let policy = node_policy::read(&root).unwrap();
    assert_eq!(policy.target, "darwin-arm64");
    assert_eq!(policy.version, env!("LENS_NODE_VERSION"));
    assert_eq!(policy.archive_url, env!("LENS_NODE_ARCHIVE_URL"));
    assert_eq!(policy.archive_sha256, env!("LENS_NODE_ARCHIVE_SHA256"));
}

#[test]
fn artifact_is_derived_without_build_host_inference() {
    let policy = node_policy::parse(CONFIG, LOCK).unwrap();
    assert_eq!(policy.version, "24.20.0");
    assert_eq!(policy.target, "darwin-arm64");
    assert_eq!(policy.archive_root, "node-v24.20.0-darwin-arm64");
    assert_eq!(policy.archive_name, "node-v24.20.0-darwin-arm64.tar.gz");
    assert_eq!(policy.archive_sha256.len(), 64);
    let next = node_policy::parse(
        &CONFIG.replace("24.20.0", "25.0.0"),
        &LOCK.replace("24.20.0", "25.0.0"),
    )
    .unwrap();
    assert_eq!(next.archive_name, "node-v25.0.0-darwin-arm64.tar.gz");
}

#[test]
fn malformed_or_ambiguous_configuration_fails_closed() {
    for config in [
        "[",
        "[tools]",
        "[tools]\nnode = ['24.20.0']",
        "[tools]\nnode = '24'",
        "[tools]\nnode = '^24.20.0'",
        "[tools]\nnode = '24.020.0'",
        "[tools]\nnode = '24.20.0-beta.1'",
    ] {
        assert!(
            node_policy::parse(config, LOCK).is_err(),
            "accepted {config}"
        );
    }
    for lock in ["[", "[tools]", "[tools]\nnode = []"] {
        assert!(node_policy::parse(CONFIG, lock).is_err(), "accepted {lock}");
    }
    assert!(node_policy::parse(
        CONFIG,
        &format!("{LOCK}\n[[tools.node]]\nversion = '24.20.0'\nbackend = 'core:node'")
    )
    .is_err());
}

#[test]
fn drift_or_unapproved_artifact_fails_closed() {
    for (from, to) in [
        ("version = \"24.20.0\"", "version = \"24.19.0\""),
        ("core:node", "asdf:nodejs"),
        ("platforms.macos-arm64", "platforms.linux-arm64"),
        ("https://nodejs.org", "http://nodejs.org"),
        ("https://nodejs.org", "https://example.org"),
        ("darwin-arm64.tar.gz", "darwin-x64.tar.gz"),
        ("tar.gz", "tar.xz"),
        ("sha256:", "sha512:"),
        ("sha256:40", "sha256:zz"),
        ("sha256:40", "sha256:"),
    ] {
        assert!(
            node_policy::parse(CONFIG, &LOCK.replace(from, to)).is_err(),
            "accepted mutation {from} => {to}"
        );
    }
    for field in ["checksum =", "url =", "backend =", "version ="] {
        let lock = LOCK
            .lines()
            .filter(|line| !line.starts_with(field))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            node_policy::parse(CONFIG, &lock).is_err(),
            "accepted missing {field}"
        );
    }
}

#[test]
fn every_manifest_requires_an_exact_matching_engine() {
    for path in &node_policy::INPUTS[2..] {
        node_policy::validate_engine(path, r#"{"engines":{"node":"24.20.0"}}"#, "24.20.0").unwrap();
        for manifest in [
            "{",
            "{}",
            r#"{"engines":{"node":"24.19.0"}}"#,
            r#"{"engines":{"node":"^24.20.0"}}"#,
            r#"{"engines":{"node":24}}"#,
        ] {
            let error = node_policy::validate_engine(path, manifest, "24.20.0").unwrap_err();
            assert!(error.contains(path));
        }
    }
}
