//! Disposable navigation probe. Only --open-browser enables the production opener.
#[path = "../src/html_preview.rs"]
mod html_preview;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tauri::http::Response;

fn main() {
    let permit = std::env::args().any(|arg| arg == "--permit-frame-navigation");
    let popup = std::env::args().any(|arg| arg == "--popup");
    let open_browser = std::env::args().any(|arg| arg == "--open-browser");
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let child = r#"<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; form-action 'none'"><style>a{display:block;padding:25px;font:24px sans-serif}</style><h2>Sandbox empty; no scripts</h2><a href="https://example.com/lens-probe">HTTPS anchor</a><a href="lens-link-probe://open/opaque-id">Opaque scheme anchor</a>"#;
    let parent = format!("<!doctype html><h1>Disposable navigation probe</h1><a href=\"https://example.com/parent-control\">Parent positive control</a><iframe sandbox=\"\" style=\"width:650px;height:380px\" srcdoc=\"{}\"></iframe>", child.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;"));
    let parent = if popup {
        parent
            .replace(
                "Sandbox empty; no scripts",
                "Sandbox allow-popups; no scripts",
            )
            .replace("sandbox=\"\"", "sandbox=\"allow-popups\"")
            .replace(
                "&lt;a href=",
                "&lt;a target=&quot;_blank&quot; rel=&quot;noopener noreferrer&quot; href=",
            )
    } else {
        parent
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("lens-link-probe", |_, _| {
            println!("OPAQUE_PROTOCOL_HANDLER_REACHED");
            Response::builder().status(403).body(Vec::<u8>::new()).unwrap()
        })
        .register_uri_scheme_protocol("navigation-parent", move |_, _| {
            Response::builder().header("Content-Type", "text/html")
                .header("Content-Security-Policy", if permit { "default-src 'self'; frame-src 'self' https: lens-link-probe:; script-src 'self'; style-src 'unsafe-inline'; connect-src ipc:" } else { "default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; connect-src ipc:" })
                .body(parent.as_bytes().to_vec()).unwrap()
        })
        .setup(move |app| {
            let opener_app = app.handle().clone();
            tauri::WebviewWindowBuilder::new(app, "lens-overlay", tauri::WebviewUrl::External("navigation-parent://document".parse()?))
                .title("Disposable Lens navigation probe")
                .inner_size(720.0, 520.0)
                .on_new_window(move |url, _| {
                    println!("POPUP_CAPTURED_AND_DENIED={url}");
                    if open_browser {
                        println!("PRODUCTION_BROWSER_HANDOFF={url}");
                        return html_preview::open_link(&opener_app, &url);
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .on_navigation(move |url| {
                    println!("NAVIGATION_OBSERVED={url}");
                    if url.scheme() == "https" || url.scheme() == "lens-link-probe" {
                        if popup && url.path() == "/lens-probe" { return true; }
                        observed.fetch_add(1, Ordering::SeqCst);
                        println!("LINK_CANCELLED={url}");
                        return false;
                    }
                    true
                })
                .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(90));
                println!("CAPTURED_LINK_COUNT={}", count.load(Ordering::SeqCst));
                handle.exit(0);
            });
            Ok(())
        })
        .build(tauri::generate_context!()).expect("build disposable navigation probe");
    std::process::exit(app.run_return(|_, _| {}));
}
