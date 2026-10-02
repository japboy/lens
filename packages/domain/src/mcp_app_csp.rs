//! Shared deterministic MCP Apps origin validation for publication and replay.
use serde_json::{json, Value};

/// Normalize the supported MCP Apps CSP fields. No unknown policy is silently ignored.
pub fn normalize_csp(csp: &Value) -> Result<Value, String> {
    let fields = csp.as_object().ok_or("Invalid App CSP metadata")?;
    if fields.keys().any(|field| {
        !matches!(
            field.as_str(),
            "resourceDomains" | "connectDomains" | "frameDomains" | "baseUriDomains"
        )
    }) {
        return Err("Unsupported App CSP field".into());
    }
    let sources = |field: &str, connect: bool| -> Result<Vec<String>, String> {
        let Some(value) = csp.get(field) else {
            return Ok(Vec::new());
        };
        let values = value
            .as_array()
            .filter(|v| v.len() <= 32)
            .ok_or("Invalid or oversized App CSP source list")?;
        values
            .iter()
            .map(|v| {
                let value = v
                    .as_str()
                    .filter(|v| v.len() <= 256)
                    .ok_or("Invalid CSP origin")?;
                let url = url::Url::parse(value)
                    .map_err(|_| "CSP sources must be explicit HTTP(S) origins")?;
                let supported_scheme = matches!(url.scheme(), "http" | "https")
                    || connect && matches!(url.scheme(), "ws" | "wss");
                if !supported_scheme
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || url.path() != "/"
                {
                    return Err("Unsupported CSP source syntax".into());
                }
                let host = url.host_str().ok_or("CSP source has no host")?;
                let host = host.strip_prefix("*.").unwrap_or(host);
                if host.is_empty()
                    || !host.bytes().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b':' | b'[' | b']')
                    })
                {
                    return Err("Invalid CSP source host".into());
                }
                Ok(url.as_str().trim_end_matches('/').to_owned())
            })
            .collect()
    };
    let resource_domains = sources("resourceDomains", false)?;
    let connect_domains = sources("connectDomains", true)?;
    if ["frameDomains", "baseUriDomains"].iter().any(|field| {
        csp.get(field)
            .is_some_and(|v| v.as_array().is_none_or(|v| !v.is_empty()))
    }) {
        return Err("External frames and base origins are unsupported".into());
    }
    Ok(json!({"resourceDomains": resource_domains, "connectDomains": connect_domains}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_normalize_without_crossing_resource_and_connection_policy() {
        assert_eq!(normalize_csp(&json!({"resourceDomains":["https://CDN.example:443/"],"connectDomains":["wss://data.example/", "https://api.example:443/"]})).unwrap(),json!({"resourceDomains":["https://cdn.example"],"connectDomains":["wss://data.example","https://api.example"]}));
        assert_eq!(
            normalize_csp(&json!({})).unwrap(),
            json!({"resourceDomains":[],"connectDomains":[]})
        );
        assert_eq!(
            normalize_csp(&json!({"frameDomains":[],"baseUriDomains":[]})).unwrap(),
            normalize_csp(&json!({})).unwrap()
        );
        assert!(normalize_csp(&json!({"resourceDomains":["wss://data.example"]})).is_err());
    }

    #[test]
    fn policy_rejects_unknown_fields_directives_paths_and_unbounded_inputs() {
        for input in [
            json!(null),
            json!({"unsupported":[]}),
            json!({"frameDomains":["https://frame.example"]}),
            json!({"baseUriDomains":null}),
            json!({"resourceDomains":true}),
            json!({"connectDomains":vec!["https://api.example";33]}),
        ] {
            assert!(normalize_csp(&input).is_err(), "{input}");
        }
        for source in [
            "*",
            "data:",
            "https://cdn.example/path",
            "https://user@cdn.example",
            "https://cdn.example?q=1",
            "https://cdn.example#anchor",
            "https://cdn.example; script-src *",
        ] {
            assert!(
                normalize_csp(&json!({"resourceDomains":[source]})).is_err(),
                "{source}"
            );
        }
    }
}
