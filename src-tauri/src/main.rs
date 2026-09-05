#[cfg(target_os = "macos")]
fn main() {
    lens_lib::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "Lens currently supports macOS only; this target is for common-library verification."
    );
    std::process::exit(1);
}
