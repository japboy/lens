#[cfg(target_os = "macos")]
fn main() {
    if let Some(status) = lens_lib::internal_process_exit() {
        std::process::exit(status);
    }
    lens_lib::run();
}

#[cfg(not(any(
    target_os = "macos",
    all(
        windows,
        target_arch = "x86_64",
        target_env = "msvc",
        feature = "windows-fixture-desktop",
        debug_assertions
    )
)))]
fn main() {
    eprintln!(
        "Lens currently supports macOS only; this target is for common-library verification."
    );
    std::process::exit(1);
}
#[cfg(all(
    windows,
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "windows-fixture-desktop",
    debug_assertions
))]
mod initial_publication;
#[cfg(all(
    windows,
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "windows-fixture-desktop",
    debug_assertions
))]
mod windows_fixture_desktop;

#[cfg(all(
    windows,
    target_arch = "x86_64",
    target_env = "msvc",
    feature = "windows-fixture-desktop",
    debug_assertions
))]
fn main() {
    if let Err(error) = windows_fixture_desktop::run() {
        eprintln!("Windows fixture Desktop startup failed: {error}");
        std::process::exit(1);
    }
}
