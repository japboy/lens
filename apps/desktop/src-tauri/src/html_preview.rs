//! Native destination policy for scripts-disabled HTML preview links.
use tauri::{webview::NewWindowResponse, AppHandle, Runtime, Url};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

fn browser_destination(url: &Url) -> Option<&str> {
    matches!(url.scheme(), "http" | "https").then_some(url.as_str())
}

/// Never create an application WebView for generated content. `allow-popups`
/// permits a declarative anchor request, not JavaScript or same-origin access.
pub(crate) fn open_link<R: Runtime>(app: &AppHandle<R>, url: &Url) -> NewWindowResponse<R> {
    if let Some(destination) = browser_destination(url) {
        if app.opener().open_url(destination, None::<&str>).is_err() {
            app.dialog()
                .message("The link could not be opened in your default browser.")
                .title("Unable to open link")
                .kind(MessageDialogKind::Error)
                .show(|_| {});
        }
    }
    NewWindowResponse::Deny
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_reach_the_operating_system() {
        for value in ["https://example.com/path?q=a#b", "http://example.com/"] {
            let url = Url::parse(value).unwrap();
            assert_eq!(browser_destination(&url), Some(value));
        }
        for value in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hi",
            "mailto:user@example.com",
            "tauri://localhost/overlay.html",
        ] {
            assert_eq!(browser_destination(&Url::parse(value).unwrap()), None);
        }
    }
}
