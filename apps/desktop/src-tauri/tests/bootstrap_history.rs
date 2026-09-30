#[path = "../bootstrap_history.rs"]
mod bootstrap_history;
#[test]
fn approved_history_is_explicit_and_unambiguous() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    assert!(!bootstrap_history::read(&root).unwrap().is_empty());
    let text = include_str!("../agent-runtime/node-history.toml");
    for (from, to) in [
        ("24.21.0", "24.021.0"),
        ("24.21.0", "^24.21.0"),
        ("sha256", "sha512"),
        ("bed7", "BEd7"),
    ] {
        assert!(bootstrap_history::parse(&text.replace(from, to)).is_err());
    }
    assert!(bootstrap_history::parse(&format!("{text}\n{text}")).is_err());
    assert!(bootstrap_history::parse("previous = 'wrong'").is_err());
}
