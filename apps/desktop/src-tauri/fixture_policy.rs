//! Admission for the development-only Windows fixture composition.
pub fn admits(os: &str, arch: &str, env: &str, profile: &str, debug_assertions: bool) -> bool {
    (os, arch, env, profile, debug_assertions) == ("windows", "x86_64", "msvc", "debug", true)
}
