fn main() {
    tauri_build::build();
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let platform_path = format!("tauri.{target}.conf.json");
    println!("cargo:rerun-if-changed={platform_path}");
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    // Preserve the build's configuration inputs; Tauri removes bundle fields from runtime Config.
    for (name, text) in [
        (
            "LENS_ABOUT_CONFIG",
            std::fs::read_to_string("tauri.conf.json").unwrap(),
        ),
        (
            "LENS_ABOUT_PLATFORM_CONFIG",
            std::fs::read_to_string(platform_path).unwrap_or_else(|_| "{}".into()),
        ),
        (
            "LENS_ABOUT_CONFIG_OVERRIDE",
            std::env::var("TAURI_CONFIG").unwrap_or_else(|_| "{}".into()),
        ),
    ] {
        println!(
            "cargo:rustc-env={name}={}",
            text.lines().collect::<Vec<_>>().join(" ")
        );
    }
    for document in ["../../../LICENSE", "../../../NOTICE"] {
        println!("cargo:rerun-if-changed={document}");
        let text = std::fs::read_to_string(document).expect("License document must exist as UTF-8");
        assert!(
            !text.trim().is_empty(),
            "License document must not be empty"
        );
    }
}
