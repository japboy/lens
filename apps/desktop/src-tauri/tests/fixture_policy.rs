#[path = "../fixture_policy.rs"]
mod fixture_policy;

#[test]
fn only_the_debug_windows_x64_msvc_fixture_is_admitted() {
    assert!(fixture_policy::admits(
        "windows", "x86_64", "msvc", "debug", true
    ));
    for (os, arch, env, profile, assertions) in [
        ("macos", "x86_64", "msvc", "debug", true),
        ("linux", "x86_64", "gnu", "debug", true),
        ("windows", "aarch64", "msvc", "debug", true),
        ("windows", "x86_64", "gnu", "debug", true),
        ("windows", "x86_64", "msvc", "release", false),
        ("windows", "x86_64", "msvc", "release", true),
        ("windows", "x86_64", "msvc", "debug", false),
        ("windows", "x86_64", "msvc", "", true),
    ] {
        assert!(!fixture_policy::admits(os, arch, env, profile, assertions));
    }
}
