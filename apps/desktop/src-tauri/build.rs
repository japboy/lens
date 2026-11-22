mod bootstrap_history;
mod fixture_policy;
mod node_policy;
mod pnpm_policy;

fn main() {
    if std::env::var_os("CARGO_FEATURE_WINDOWS_FIXTURE_DESKTOP").is_some() {
        assert!(
            fixture_policy::admits(
                &std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default(),
                &std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default(),
                &std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default(),
                &std::env::var("PROFILE").unwrap_or_default(),
                std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some(),
            ),
            "windows-fixture-desktop requires a Windows x64 MSVC debug build with debug assertions"
        );
        println!("cargo:rerun-if-changed=tauri.fixture.conf.json");
        println!("cargo:rerun-if-changed=fixture-webview");
        let config: serde_json::Value =
            serde_json::from_str(include_str!("tauri.fixture.conf.json")).expect("fixture config");
        println!("cargo:rustc-env=TAURI_CONFIG={config}");
        // The fixture uses the embedded local WebView and app-local commands only.
        // Keep the product permissions and generated schemas out of this composition.
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("build output"));
        for file in ["acl-manifests.json", "capabilities.json"] {
            std::fs::write(out.join(file), "{}").expect("empty fixture capability manifest");
        }
        // Register Tauri's core API for the embedded page without generating product capabilities.
        let global_api = std::env::var("DEP_TAURI_GLOBAL_API_SCRIPT_PATH")
            .expect("Tauri must provide its global API bundle");
        println!("cargo:rerun-if-changed={global_api}");
        let scripts = serde_json::to_vec(&[global_api]).expect("global API script paths");
        std::fs::write(out.join("__global-api-script.js"), scripts)
            .expect("fixture global API registration");
        // The Windows loader needs Common Controls v6 for TaskDialogIndirect.
        println!("cargo:rustc-link-arg-bin=lens=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=lens=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
        return;
    }
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../..");
    for input in node_policy::INPUTS {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    let policy =
        node_policy::read(&root).expect("managed Node policy must match the development lock");
    for (name, value) in [
        ("LENS_NODE_VERSION", policy.version.as_str()),
        ("LENS_NODE_TARGET", policy.target),
        ("LENS_NODE_ARCHIVE_NAME", policy.archive_name.as_str()),
        ("LENS_NODE_ARCHIVE_ROOT", policy.archive_root.as_str()),
        ("LENS_NODE_ARCHIVE_URL", policy.archive_url.as_str()),
        ("LENS_NODE_ARCHIVE_SHA256", policy.archive_sha256.as_str()),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }
    for input in pnpm_policy::INPUTS
        .into_iter()
        .chain([pnpm_policy::HISTORY_INPUT])
    {
        println!("cargo:rerun-if-changed={}", root.join(input).display());
    }
    let pnpm = pnpm_policy::read(&root)
        .expect("managed pnpm must match the development version and native lock");
    for (name, value) in [
        ("LENS_PNPM_VERSION", pnpm.version.as_str()),
        ("LENS_PNPM_ARCHIVE_NAME", pnpm.archive_name.as_str()),
        ("LENS_PNPM_ARCHIVE_URL", pnpm.archive_url.as_str()),
        ("LENS_PNPM_ARCHIVE_SHA256", pnpm.archive_sha256.as_str()),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }
    println!(
        "cargo:rerun-if-changed={}",
        root.join(bootstrap_history::INPUT).display()
    );
    let mut node_identities = bootstrap_history::read(&root)
        .expect("Node history must contain exact approved identities");
    for (version, digest) in &node_identities {
        assert!(
            version != &policy.version || digest == &policy.archive_sha256,
            "conflicting current and historical Node digest"
        );
    }
    node_identities.push((policy.version, policy.archive_sha256));
    let mut pnpm_identities = pnpm.previous;
    pnpm_identities.push((pnpm.version, format!("sha256:{}", pnpm.archive_sha256)));
    for (name, identities) in [
        ("LENS_APPROVED_NODE_IDENTITIES", node_identities),
        ("LENS_APPROVED_PNPM_IDENTITIES", pnpm_identities),
    ] {
        println!(
            "cargo:rustc-env={name}={}",
            serde_json::to_string(&identities).unwrap()
        );
    }
    tauri_build::build();
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target == "macos" {
        // ScreenCaptureKit uses the OS Swift runtime. Dependency link arguments
        // do not propagate to this final executable or its test targets.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
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
