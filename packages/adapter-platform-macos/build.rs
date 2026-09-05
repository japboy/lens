fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    cc::Build::new()
        .file("native/LensNative.m")
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
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    println!("cargo:rerun-if-changed=native/LensNative.m");
    println!("cargo:rerun-if-changed=native/LensNative.h");
}
