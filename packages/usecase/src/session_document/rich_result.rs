//! Validation of a typed MCP result flattened into receipt and structured-body text.
//! No saved-file hint is opened: complete input and the receipt remain the authority.

use serde_json::Value;

/// Grok joins MCP text blocks and compact structuredContent with a newline, and
/// may truncate the resulting byte prefix. Accept only the exact publication
/// body derived from already-validated arguments and its one complete receipt.
pub(super) fn concatenated_publication(
    text: &str,
    html: &str,
    csp: Option<&Value>,
    digest: &str,
    csp_digest: Option<&str>,
) -> bool {
    let mut values = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let Some(Ok(receipt)) = values.next() else {
        return false;
    };
    let offset = values.byte_offset();
    let compact_receipt = receipt.to_string();
    if super::failed_tool_result(&receipt)
        || !super::rich_receipt(&receipt, digest, csp_digest)
        || text[..offset] != compact_receipt
    {
        return false;
    }
    let Some(body_text) = text[offset..].strip_prefix('\n') else {
        return false;
    };
    let mut body = serde_json::json!({"html": html, "publication": receipt});
    if let Some(csp) = csp {
        body["csp"] = csp.clone();
    }
    let expected_body = extracted_data_uri_echo(body.to_string());
    if body_text == expected_body {
        return true;
    }
    let Some((body_prefix, footer)) =
        body_text.rsplit_once("\n\n[MCP output truncated: showing first ")
    else {
        return false;
    };
    if body_prefix.is_empty()
        || body_prefix.len() >= expected_body.len()
        || !expected_body.starts_with(body_prefix)
        || footer.contains(['\n', '\r'])
    {
        return false;
    }
    let shown_bytes = offset + 1 + body_prefix.len();
    let total_bytes = offset + 1 + expected_body.len();
    // The formatter reports its byte budget; UTF-8 truncation may back up by
    // fewer bytes than the next code point to avoid splitting it.
    let next_character_bytes = expected_body[body_prefix.len()..]
        .chars()
        .next()
        .unwrap()
        .len_utf8();
    (shown_bytes..shown_bytes + next_character_bytes).any(|budget| {
        if budget >= total_bytes {
            return false;
        }
        let prefix = format!("{} of {}.", format_bytes(budget), format_bytes(total_bytes));
        footer.strip_prefix(&prefix).is_some_and(|hint| {
            hint == "]" || (hint.starts_with(" Full output written to: ") && hint.ends_with(']'))
        })
    })
}

/// Grok's base64_images.rs rewrites serialized tool text before truncation:
/// https://github.com/xai-org/grok-build/blob/2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8/crates/codegen/xai-grok-tools/src/util/base64_images.rs
/// Canonical JSON has no raw CR/LF, and MAX_HTML keeps payloads below its 10 MiB cap.
fn extracted_data_uri_echo(mut text: String) -> String {
    for pdf in [true, false] {
        // Preserve the provider's case-sensitive image fast path after PDF removal.
        if !pdf && !text.contains("data:image") {
            continue;
        }
        let prefixes = data_uri_prefixes(&text, pdf);
        let mut boundaries = data_uri_prefixes(&text, !pdf);
        boundaries.extend(prefixes.iter().copied());
        boundaries.sort_unstable();
        let mut result = String::with_capacity(text.len());
        let mut last_end = 0;
        let mut images = 0;
        for (start, payload_start) in prefixes {
            let cap = boundaries
                .get(boundaries.partition_point(|(next, _)| *next <= start))
                .map_or(text.len(), |(next, _)| *next);
            let payload_len = text.as_bytes()[payload_start..cap]
                .iter()
                .take_while(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                })
                .count();
            if !pdf && payload_len - payload_len % 4 < 1024 {
                continue;
            }
            result.push_str(&text[last_end..start]);
            if pdf {
                result.push_str(&format!(
                    "[PDF attachment removed — {} KB]",
                    payload_len * 3 / 4 / 1024
                ));
            } else if images < 5 {
                result.push_str("[image content will be provided separately]");
                images += 1;
            } else {
                result.push_str("[additional image omitted]");
            }
            last_end = payload_start + payload_len;
        }
        result.push_str(&text[last_end..]);
        text = result;
    }
    text
}

/// Preserve the provider's exact regexes, including Unicode case folding.
fn data_uri_prefixes(text: &str, pdf: bool) -> Vec<(usize, usize)> {
    use regex::Regex;
    use std::sync::LazyLock;
    static IMAGE_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:[^a-zA-Z0-9]|^)(data:(image/(?:png|jpeg|gif|webp|bmp|tiff))(?:;[^\s,;]{1,120})*;base64,)").expect("constant image prefix")
    });
    static PDF_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:[^a-zA-Z0-9]|^)(data:application/pdf(?:;[^\s,;]{1,120})*;base64,)")
            .expect("constant PDF prefix")
    });
    let prefix = if pdf { &*PDF_PREFIX } else { &*IMAGE_PREFIX };
    prefix
        .captures_iter(text)
        .filter_map(|captures| captures.get(1).map(|value| (value.start(), value.end())))
        .collect()
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    for unit in ["KB", "MB", "GB", "TB", "PB"] {
        if value < 1023.95 {
            return format!("{value:.1} {unit}");
        }
        value /= 1024.0;
    }
    format!("{value:.1} EB")
}

#[cfg(test)]
mod tests {
    use super::extracted_data_uri_echo;

    #[test]
    fn data_uri_echo_matches_provider_mime_size_case_and_count_boundaries() {
        let placeholder = "[image content will be provided separately]";
        for mime in ["png", "jpeg", "gif", "webp", "bmp", "tiff"] {
            let uri = format!(
                "data:image/{mime};charset=utf-8;Base64,{}",
                "A".repeat(1025)
            );
            let body = serde_json::json!({"html":format!("<img src='{uri}'>")}).to_string();
            assert_eq!(
                extracted_data_uri_echo(body.clone()),
                body.replace(&uri, placeholder)
            );
        }
        for uri in [
            format!(
                "data:image/png;{};base64,{}",
                "q".repeat(120),
                "A".repeat(1024)
            ),
            format!("data:image/png;baſe64,{}", "A".repeat(1024)),
        ] {
            let body = serde_json::json!({"html":uri}).to_string();
            assert_eq!(
                extracted_data_uri_echo(body.clone()),
                body.replace(&uri, placeholder)
            );
        }
        for uri in [
            "data:image/png;base64,".to_owned(),
            format!("data:image/png;base64,{}", "A".repeat(1023)),
            format!("data:image/svg+xml;base64,{}", "A".repeat(1024)),
            format!("data:image/x-icon;base64,{}", "A".repeat(1024)),
            format!("metadata:image/png;base64,{}", "A".repeat(1024)),
            format!("Kdata:image/png;base64,{}", "A".repeat(1024)),
            format!("ſdata:image/png;base64,{}", "A".repeat(1024)),
            format!("DATA:IMAGE/PNG;BASE64,{}", "A".repeat(1024)),
            format!(
                "data:image/png;{};base64,{}",
                "q".repeat(121),
                "A".repeat(1024)
            ),
        ] {
            let body = serde_json::json!({"html":uri}).to_string();
            assert_eq!(extracted_data_uri_echo(body.clone()), body);
        }
        let uppercase = format!("DATA:IMAGE/PNG;BASE64,{}", "A".repeat(1024));
        let mixed = format!("data:image unmatched {uppercase}");
        assert_eq!(
            extracted_data_uri_echo(mixed),
            format!("data:image unmatched {placeholder}")
        );
        let uri = format!("data:image/png;base64,{}", "A".repeat(1024));
        let six = std::iter::repeat_n(uri.as_str(), 6)
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            extracted_data_uri_echo(six),
            format!(
                "{} [additional image omitted]",
                std::iter::repeat_n(placeholder, 5)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        );
        let pdf = format!("data:Application/PDF;Base64,{}", "A".repeat(2048));
        assert_eq!(
            extracted_data_uri_echo(pdf),
            "[PDF attachment removed — 1 KB]"
        );
    }
}
