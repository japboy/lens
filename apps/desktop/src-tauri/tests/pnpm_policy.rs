#[path = "../pnpm_policy.rs"]
mod pnpm_policy;
const CONFIG: &str = include_str!("../agent-runtime/pnpm.toml");
#[test]
fn one_declaration_derives_all_archive_identifiers_and_digest() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let policy = pnpm_policy::read(&root).unwrap();
    assert_eq!(policy.version, env!("LENS_PNPM_VERSION"));
    assert_eq!(policy.archive_sha512, env!("LENS_PNPM_ARCHIVE_SHA512"));
    assert_eq!(policy.archive_name, env!("LENS_PNPM_ARCHIVE_NAME"));
    assert_eq!(policy.archive_url, env!("LENS_PNPM_ARCHIVE_URL"));
    let next = pnpm_policy::parse(&CONFIG.replace("11.22.0", "11.23.0")).unwrap();
    assert_eq!(
        next.archive_url,
        "https://npm.flatt.tech/pnpm/-/pnpm-11.23.0.tgz"
    );
    assert_eq!(next.archive_name, "pnpm-11.23.0.tgz");
}
#[test]
fn noncanonical_or_incompatible_policies_fail_closed() {
    for (from, to) in [
        ("11.22.0", "12.0.0"),
        ("11.22.0", "11.022.0"),
        ("11.22.0", "^11.22.0"),
        ("11.22.0", "11.22.0-beta.1"),
        ("sha512-", "sha256-"),
        ("WIQ==", "WIQ="),
        ("H/hw", "!!!!"),
        ("WIQ==", "WIQ== extra"),
    ] {
        assert!(
            pnpm_policy::parse(&CONFIG.replace(from, to)).is_err(),
            "{from} => {to}"
        );
    }
    assert!(pnpm_policy::parse(&format!("{CONFIG}\nurl = 'https://example.com'\n")).is_err());
    assert!(pnpm_policy::parse("version='11.22.0'").is_err());
}
#[test]
fn previous_declarations_are_load_only_and_exact() {
    let history = CONFIG
        .replace("11.22.0", "11.21.0")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let policy = pnpm_policy::parse(&format!("{CONFIG}\n[[previous]]\n{history}")).unwrap();
    assert_eq!(
        policy.previous,
        vec![("11.21.0".into(), policy.archive_sha512.clone())]
    );
    assert!(pnpm_policy::parse(&format!(
        "{CONFIG}\n[[previous]]\n{history}\n[[previous]]\n{history}"
    ))
    .is_err());
    assert!(pnpm_policy::parse(&format!(
        "{CONFIG}\n[[previous]]\nversion='12.0.0'\nintegrity='wrong'"
    ))
    .is_err());
}
