use crate::{app_state::AppState, ui::LENS_WINDOW_LABEL};
use base64::prelude::*;
use tauri::{
    http::{header, Method, Request, Response, StatusCode, Uri},
    Manager, Runtime, UriSchemeContext,
};

pub const LENS_MEDIA_SCHEME: &str = "personallens";

pub fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let state = context.app_handle().state::<AppState>();
    response(
        &state,
        context.webview_label(),
        request.method(),
        request.uri(),
    )
}

fn response(
    state: &AppState,
    webview_label: &str,
    method: &Method,
    uri: &Uri,
) -> Response<Vec<u8>> {
    if webview_label != LENS_WINDOW_LABEL {
        return error_response(
            StatusCode::FORBIDDEN,
            "Lens media preview is not available here",
        );
    }
    if method != Method::GET {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, Method::GET.as_str())
            .header(header::CACHE_CONTROL, "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(b"Lens media preview requires GET".to_vec())
            .expect("static Lens media response must be valid");
    }

    let payload = match state.lens_media.payload_for_uri(&uri.to_string()) {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "Lens media preview is not available for this operation",
            );
        }
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Lens media preview state is unavailable",
            );
        }
    };
    if payload.mime_type != "image/png" {
        return error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Lens media preview has an unsupported type",
        );
    }
    let data = match BASE64_STANDARD.decode(&payload.data) {
        Ok(data) => data,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Lens media preview payload is invalid",
            );
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, payload.mime_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(data)
        .expect("validated Lens media response must be valid")
}

fn error_response(status: StatusCode, message: &'static str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.as_bytes().to_vec())
        .expect("static Lens media error response must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens::LensMediaPayload;
    use uuid::Uuid;

    fn active_state(uri: &str, mime_type: &str, data: &str) -> AppState {
        let state = AppState::load();
        let operation_id = Uuid::new_v4();
        state
            .lens_media
            .begin(operation_id)
            .expect("begin media operation");
        state
            .lens_media
            .replace(
                operation_id,
                vec![LensMediaPayload {
                    attachment_id: "media-node-000001".into(),
                    uri: uri.into(),
                    mime_type: mime_type.into(),
                    data: data.into(),
                }],
            )
            .expect("store media payload");
        state
    }

    #[test]
    fn serves_only_the_exact_active_png_to_the_lens_webview() {
        let uri =
            "personallens://context/00000000-0000-0000-0000-000000000001/1/media/media-node-000001";
        let state = active_state(uri, "image/png", "iVBORw0KGgo=");
        let served = response(
            &state,
            LENS_WINDOW_LABEL,
            &Method::GET,
            &uri.parse().expect("valid URI"),
        );

        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(served.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(served.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(served.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(
            served.body(),
            &BASE64_STANDARD.decode("iVBORw0KGgo=").unwrap()
        );

        let missing = response(
            &state,
            LENS_WINDOW_LABEL,
            &Method::GET,
            &"personallens://context/other/1/media/media-node-000001"
                .parse()
                .expect("valid URI"),
        );
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn rejects_other_webviews_methods_types_and_invalid_payloads() {
        let uri =
            "personallens://context/00000000-0000-0000-0000-000000000001/1/media/media-node-000001";
        let png = active_state(uri, "image/png", "iVBORw0KGgo=");
        assert_eq!(
            response(
                &png,
                "settings",
                &Method::GET,
                &uri.parse().expect("valid URI")
            )
            .status(),
            StatusCode::FORBIDDEN
        );
        let method = response(
            &png,
            LENS_WINDOW_LABEL,
            &Method::POST,
            &uri.parse().expect("valid URI"),
        );
        assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(method.headers()[header::ALLOW], "GET");

        let unsupported = active_state(uri, "image/svg+xml", "PHN2Zz4=");
        assert_eq!(
            response(
                &unsupported,
                LENS_WINDOW_LABEL,
                &Method::GET,
                &uri.parse().expect("valid URI")
            )
            .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );

        let invalid = active_state(uri, "image/png", "not-base64");
        assert_eq!(
            response(
                &invalid,
                LENS_WINDOW_LABEL,
                &Method::GET,
                &uri.parse().expect("valid URI")
            )
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn superseding_the_operation_revokes_the_previous_uri() {
        let uri =
            "personallens://context/00000000-0000-0000-0000-000000000001/1/media/media-node-000001";
        let state = active_state(uri, "image/png", "iVBORw0KGgo=");
        state
            .lens_media
            .begin(Uuid::new_v4())
            .expect("supersede operation");

        assert_eq!(
            response(
                &state,
                LENS_WINDOW_LABEL,
                &Method::GET,
                &uri.parse().expect("valid URI")
            )
            .status(),
            StatusCode::NOT_FOUND
        );
    }
}
