fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    // The safe ScreenCaptureKit dependency statically links its Swift bridge.
    // Cargo does not propagate a dependency\'s rustc-link-arg to this test binary.
    // Resolve Swift libraries from the OS dyld cache, never a developer toolchain.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    cc::Build::new()
        .file("native/LensNative.m")
        .file("native/LensControlPalette.m")
        .include("native")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=15.2")
        .compile("lens_native");

    for framework in [
        "AppKit",
        "ApplicationServices",
        "CoreFoundation",
        "CoreGraphics",
        "Foundation",
        "ScreenCaptureKit",
        "WebKit",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rerun-if-changed=native/LensNative.m");
    println!("cargo:rerun-if-changed=native/LensNative.h");
    println!("cargo:rerun-if-changed=native/LensControlPalette.m");
}
