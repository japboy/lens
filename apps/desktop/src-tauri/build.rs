mod bootstrap_history;
mod node_policy;
mod pnpm_policy;

fn main() {
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
    println!(
        "cargo:rerun-if-changed={}",
        root.join(pnpm_policy::INPUT).display()
    );
    let pnpm =
        pnpm_policy::read(&root).expect("managed pnpm policy must name an approved npm tarball");
    for (name, value) in [
        ("LENS_PNPM_VERSION", pnpm.version.as_str()),
        ("LENS_PNPM_ARCHIVE_NAME", pnpm.archive_name.as_str()),
        ("LENS_PNPM_ARCHIVE_URL", pnpm.archive_url.as_str()),
        ("LENS_PNPM_ARCHIVE_SHA512", pnpm.archive_sha512.as_str()),
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
    for (version, digest) in &pnpm_identities {
        assert!(
            version != &pnpm.version || digest == &pnpm.archive_sha512,
            "conflicting current and historical pnpm digest"
        );
    }
    pnpm_identities.push((pnpm.version, pnpm.archive_sha512));
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
