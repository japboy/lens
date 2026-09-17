//! Read-only public math resources, independent of private Lens media authority.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tauri::{
    http::{header, Method, Request, Response, StatusCode, Uri},
    Runtime, UriSchemeContext,
};

pub const SCHEME: &str = "lens-math";
const MANIFEST: &str = "html-math-assets.json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    resource_digest: String,
    stylesheet_path: String,
    font_paths: Vec<String>,
    files: Vec<Asset>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    path: String,
    sha256: String,
    mime: String,
    byte_length: usize,
}

pub fn handle<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let resolver = context.app_handle().asset_resolver();
    response(request.method(), request.uri(), |path| {
        resolver.get(path.to_owned()).map(|asset| asset.bytes)
    })
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn response(
    method: &Method,
    uri: &Uri,
    get: impl Fn(&str) -> Option<Vec<u8>>,
) -> Response<Vec<u8>> {
    if method != Method::GET {
        return failure(StatusCode::METHOD_NOT_ALLOWED);
    }
    if uri.scheme_str() != Some(SCHEME)
        || uri.authority().map(|value| value.as_str()) != Some("assets")
        || uri.query().is_some()
    {
        return failure(StatusCode::NOT_FOUND);
    }
    let path = uri.path().strip_prefix('/').unwrap_or("");
    if !path.starts_with("assets/html-math/")
        || path.contains('%')
        || path
            .split('/')
            .any(|part| part == "." || part == ".." || part.is_empty())
    {
        return failure(StatusCode::NOT_FOUND);
    }
    let Some(manifest) = get(MANIFEST)
        .filter(|bytes| bytes.len() <= 16384)
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
    else {
        return failure(StatusCode::NOT_FOUND);
    };
    if !hash(&manifest.resource_digest)
        || manifest.files.len() != 21
        || manifest.font_paths.len() != 20
    {
        return failure(StatusCode::NOT_FOUND);
    }
    let prefix = format!("assets/html-math/{}/", manifest.resource_digest);
    let stylesheet = format!("{prefix}katex.css");
    if manifest.stylesheet_path != stylesheet {
        return failure(StatusCode::NOT_FOUND);
    }
    let Some(asset) = manifest.files.iter().find(|asset| asset.path == path) else {
        return failure(StatusCode::NOT_FOUND);
    };
    let mime = if path == stylesheet && asset.mime == "text/css" {
        "text/css; charset=utf-8"
    } else if path.starts_with(&format!("{prefix}fonts/KaTeX_"))
        && path.ends_with(".woff2")
        && asset.mime == "font/woff2"
        && manifest.font_paths.iter().any(|font| font == path)
    {
        "font/woff2"
    } else {
        return failure(StatusCode::NOT_FOUND);
    };
    if !hash(&asset.sha256) || asset.byte_length > 512 * 1024 {
        return failure(StatusCode::NOT_FOUND);
    }
    let Some(bytes) = get(path) else {
        return failure(StatusCode::NOT_FOUND);
    };
    // Tauri's resolver can fall back to index.html. The trusted manifest digest
    // makes missing, stale, or fallback bytes fail closed, never an HTML response.
    if bytes.len() != asset.byte_length || digest(&bytes) != asset.sha256 {
        return failure(StatusCode::NOT_FOUND);
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header("X-Content-Type-Options", "nosniff")
        .body(bytes)
        .expect("static math response headers are valid")
}
fn failure(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::ALLOW, "GET")
        .header("X-Content-Type-Options", "nosniff")
        .body(b"Math asset unavailable".to_vec())
        .expect("static response headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (String, Vec<u8>, Vec<u8>) {
        let content = b".katex{color:inherit}".to_vec();
        let resource_digest = "a".repeat(64);
        let path = format!("assets/html-math/{resource_digest}/katex.css");
        let fonts = (0..20)
            .map(|n| format!("assets/html-math/{resource_digest}/fonts/KaTeX_{n}.woff2"))
            .collect::<Vec<_>>();
        let mut files = vec![
            serde_json::json!({"path":path,"sha256":digest(&content),"mime":"text/css","byteLength":content.len()}),
        ];
        files.extend(fonts.iter().map(|font| serde_json::json!({"path":font,"sha256":digest(b"wOF2"),"mime":"font/woff2","byteLength":4})));
        let manifest = serde_json::to_vec(&serde_json::json!({"resourceDigest":resource_digest,"stylesheetPath":path,"fontPaths":fonts,"files":files})).unwrap();
        (path, manifest, content)
    }
    #[test]
    fn serves_only_verified_manifest_assets_with_public_cors() {
        let (path, manifest, content) = fixture();
        let result = response(
            &Method::GET,
            &format!("lens-math://assets/{path}").parse().unwrap(),
            |key| {
                if key == MANIFEST {
                    Some(manifest.clone())
                } else if key == path {
                    Some(content.clone())
                } else {
                    None
                }
            },
        );
        assert_eq!(result.status(), StatusCode::OK);
        assert_eq!(result.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            result.headers()[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        assert_eq!(result.body(), &content);
    }
    #[test]
    fn rejects_unlisted_paths_origins_queries_methods_and_fallback_html() {
        let (path, manifest, _) = fixture();
        for uri in [
            format!("lens-math://other/{path}"),
            format!("lens-math://assets/{path}?x"),
            "lens-math://assets/private/session".into(),
            "lens-math://assets/assets/html-math/%2e%2e/secret".into(),
        ] {
            let result = response(&Method::GET, &uri.parse().unwrap(), |_| {
                panic!("invalid request must not access resolver")
            });
            assert_eq!(result.status(), StatusCode::NOT_FOUND);
            assert!(!result
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        }
        assert_eq!(
            response(
                &Method::POST,
                &format!("lens-math://assets/{path}").parse().unwrap(),
                |_| None
            )
            .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        let result = response(
            &Method::GET,
            &format!("lens-math://assets/{path}").parse().unwrap(),
            |key| {
                if key == MANIFEST {
                    Some(manifest.clone())
                } else {
                    Some(b"<html>fallback</html>".to_vec())
                }
            },
        );
        assert_eq!(result.status(), StatusCode::NOT_FOUND);
    }
    #[test]
    fn verifies_fonts_and_rejects_stale_manifest_metadata() {
        let (_, manifest, _) = fixture();
        let value: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        let font = value["fontPaths"][0].as_str().unwrap();
        let uri = format!("lens-math://assets/{font}").parse().unwrap();
        let result = response(&Method::GET, &uri, |key| {
            if key == MANIFEST {
                Some(manifest.clone())
            } else {
                Some(b"wOF2".to_vec())
            }
        });
        assert_eq!(result.status(), StatusCode::OK);
        assert_eq!(result.headers()[header::CONTENT_TYPE], "font/woff2");
        for field in ["sha256", "mime", "byteLength"] {
            let mut corrupted = value.clone();
            corrupted["files"][1][field] = if field == "byteLength" {
                serde_json::json!(5)
            } else {
                serde_json::json!("invalid")
            };
            let bytes = serde_json::to_vec(&corrupted).unwrap();
            let result = response(&Method::GET, &uri, |key| {
                if key == MANIFEST {
                    Some(bytes.clone())
                } else {
                    Some(b"wOF2".to_vec())
                }
            });
            assert_eq!(result.status(), StatusCode::NOT_FOUND);
            assert!(!result
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
        }
    }
    #[test]
    fn preserves_explicit_inline_style_policy_without_disabling_script_hardening() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["app"]["security"]["dangerousDisableAssetCspModification"],
            serde_json::json!(["style-src"])
        );
        let csp = config["app"]["security"]["csp"].as_str().unwrap();
        assert!(csp.contains("style-src 'self' 'unsafe-inline'"));
        assert!(!csp.contains("script-src 'unsafe-inline'"));
        assert!(include_str!("../../src/html-output.ts").contains("script-src 'none'"));
    }
}
