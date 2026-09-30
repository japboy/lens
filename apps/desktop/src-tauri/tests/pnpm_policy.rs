#[path = "../pnpm_policy.rs"]
mod pnpm_policy;
const CONFIG: &str = "[tools]\npnpm='12.6.0'";
const LOCK: &str = r#"
[[tools.pnpm]]
version="12.6.0"
backend="aqua:pnpm/pnpm"
[tools.pnpm."platforms.macos-arm64"]
url="https://github.com/pnpm/pnpm/releases/download/v12.6.0/pnpm-darwin-arm64.tar.gz"
checksum="sha256:1030f38e14fa2e6c87fe6ab0313a3bf3b794a961504f4439bad2ca2110d3583e"
"#;
const HISTORY: &str = include_str!("../agent-runtime/pnpm-history.toml");
#[test]
fn development_identity_owns_native_archive_and_both_package_declarations() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let policy = pnpm_policy::read(&root).unwrap();
    assert_eq!(policy.version, env!("LENS_PNPM_VERSION"));
    assert_eq!(policy.archive_sha256, env!("LENS_PNPM_ARCHIVE_SHA256"));
    assert_eq!(policy.archive_name, "pnpm-darwin-arm64.tar.gz");
    assert_eq!(policy.archive_url, env!("LENS_PNPM_ARCHIVE_URL"));
    let next = pnpm_policy::parse(
        &CONFIG.replace("12.6.0", "12.7.0"),
        &LOCK.replace("12.6.0", "12.7.0"),
    )
    .unwrap();
    assert_eq!(
        next.archive_url,
        "https://github.com/pnpm/pnpm/releases/download/v12.7.0/pnpm-darwin-arm64.tar.gz"
    );
    assert!(!next.previous.iter().any(|(v, _)| v == "11.22.0"));
}
#[test]
fn mismatched_or_untrusted_native_inputs_fail_closed() {
    for version in ["11.22.0", "13.0.0", "12.06.0", "^12.6.0", "12.6.0-beta.1"] {
        assert!(pnpm_policy::parse(
            &CONFIG.replace("12.6.0", version),
            &LOCK.replace("12.6.0", version)
        )
        .is_err());
    }
    assert!(pnpm_policy::parse(&CONFIG.replace("12.6.0", "12.7.0"), LOCK).is_err());
    for (from, to) in [
        ("aqua:pnpm/pnpm", "npm:pnpm"),
        (
            "https://github.com/pnpm/pnpm/releases/",
            "https://example.com/pnpm/",
        ),
        ("pnpm-darwin-arm64.tar.gz", "pnpm-darwin-x64.tar.gz"),
        ("sha256:1030", "sha512:1030"),
        ("1030f38e", "1030F38e"),
    ] {
        assert!(
            pnpm_policy::parse(CONFIG, &LOCK.replace(from, to)).is_err(),
            "{from} => {to}"
        );
    }
    let current = r#"{"engines":{"pnpm":"12.6.0"},"packageManager":"pnpm@12.6.0"}"#;
    assert!(pnpm_policy::validate_package("fixture", current, "12.6.0").is_ok());
    for text in [
        current.replace("pnpm@12.6.0", "pnpm@11.22.0"),
        current.replace("\"pnpm\":\"12.6.0\"", "\"pnpm\":\"12.7.0\""),
    ] {
        assert!(pnpm_policy::validate_package("fixture", &text, "12.6.0").is_err());
    }
}
#[test]
fn historical_javascript_and_native_identities_are_exact_and_load_only() {
    let prior = pnpm_policy::parse_history(HISTORY).unwrap();
    assert_eq!(prior[0].0, "11.22.0");
    assert!(prior[0].1.starts_with("sha512:"));
    let native = format!(
        "[[previous]]\nversion='12.5.0'\narchive_digest='sha256:{}'\n",
        "a".repeat(64)
    );
    assert_eq!(
        pnpm_policy::parse_history(&native).unwrap(),
        vec![("12.5.0".into(), format!("sha256:{}", "a".repeat(64)))]
    );
    for text in [
        format!("{HISTORY}\n{HISTORY}"),
        native.replace("sha256:", "sha512:"),
        native.replace("12.5.0", "13.0.0"),
        native.replace("archive_digest", "integrity"),
        format!("version='12.6.0'\n{HISTORY}"),
    ] {
        assert!(pnpm_policy::parse_history(&text).is_err());
    }
}
