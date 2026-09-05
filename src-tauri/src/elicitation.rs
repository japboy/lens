//! Deliberately bounded form admission and response validation. Unsupported constraints fail closed.
use serde_json::Value;

pub fn validate_schema(schema: &Value) -> Result<(), String> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("Unsupported form schema".into());
    }
    let fields = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("Missing form fields")?;
    if fields.len() > 32
        || serde_json::to_vec(schema)
            .map_err(|_| "Invalid form")?
            .len()
            > 32 * 1024
    {
        return Err("Form exceeds the input budget".into());
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or("Invalid required fields")?;
        let mut names = std::collections::BTreeSet::new();
        for name in required {
            let name = name.as_str().ok_or("Invalid required field")?;
            if !fields.contains_key(name) || !names.insert(name) {
                return Err("Invalid required fields".into());
            }
        }
    }
    for field in fields.values() {
        let kind = field
            .get("type")
            .and_then(Value::as_str)
            .ok_or("Missing field type")?;
        if !matches!(kind, "string" | "number" | "integer" | "boolean" | "array") {
            return Err("Unsupported form field type".into());
        }
        // Pattern and format validation require a separately admitted validator; never ignore them.
        if field.get("pattern").is_some() || field.get("format").is_some() {
            return Err("This form's string constraints are not supported".into());
        }
        if field
            .get("minimum")
            .and_then(Value::as_f64)
            .zip(field.get("maximum").and_then(Value::as_f64))
            .is_some_and(|(min, max)| min > max)
            || field
                .get("minLength")
                .and_then(Value::as_u64)
                .zip(field.get("maxLength").and_then(Value::as_u64))
                .is_some_and(|(min, max)| min > max)
            || field
                .get("minItems")
                .and_then(Value::as_u64)
                .zip(field.get("maxItems").and_then(Value::as_u64))
                .is_some_and(|(min, max)| min > max)
        {
            return Err("Invalid form bounds".into());
        }
        if kind == "array" {
            let items = field.get("items").ok_or("Missing form choices")?;
            if items.get("type").and_then(Value::as_str) != Some("string")
                || enum_values(items)?.is_none()
            {
                return Err("Unsupported multiple-choice schema".into());
            }
        } else {
            let _ = enum_values(field)?;
        }
    }
    Ok(())
}
fn enum_values(field: &Value) -> Result<Option<Vec<&str>>, String> {
    let choices = if let Some(values) = field.get("enum") {
        Some(
            values
                .as_array()
                .ok_or("Invalid form choices")?
                .iter()
                .map(|v| v.as_str().ok_or("Invalid form choice"))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else if let Some(values) = field.get("oneOf") {
        Some(
            values
                .as_array()
                .ok_or("Invalid form choices")?
                .iter()
                .map(|v| {
                    v.get("const")
                        .and_then(Value::as_str)
                        .ok_or("Invalid titled form choice")
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    if let Some(choices) = &choices {
        if choices.is_empty()
            || choices.len() > 128
            || choices
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != choices.len()
        {
            return Err("Ambiguous form choices".into());
        }
    }
    Ok(choices)
}
pub fn validate_content(schema: &Value, content: &Value) -> Result<(), String> {
    validate_schema(schema)?;
    if serde_json::to_vec(content)
        .map_err(|_| "Invalid form response")?
        .len()
        > 32 * 1024
    {
        return Err("Form response exceeds the input budget".into());
    }
    let fields = schema["properties"]
        .as_object()
        .ok_or("Invalid form fields")?;
    let values = content
        .as_object()
        .ok_or("Form response must be an object")?;
    if values.keys().any(|name| !fields.contains_key(name)) {
        return Err("Unexpected form field".into());
    }
    if schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(|required| {
            required
                .iter()
                .any(|name| !values.contains_key(name.as_str().unwrap_or("")))
        })
    {
        return Err("Complete every required field".into());
    }
    for (name, value) in values {
        let field = &fields[name];
        let valid = match field["type"].as_str() {
            Some("string") => value.as_str().is_some_and(|text| {
                let length = text.chars().count() as u64;
                field
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .is_none_or(|min| length >= min)
                    && field
                        .get("maxLength")
                        .and_then(Value::as_u64)
                        .is_none_or(|max| length <= max)
                    && enum_values(field)
                        .ok()
                        .flatten()
                        .is_none_or(|choices| choices.contains(&text))
            }),
            Some("number" | "integer") => value.as_f64().is_some_and(|number| {
                number.is_finite()
                    && (field["type"] != "integer" || value.as_i64().is_some())
                    && field
                        .get("minimum")
                        .and_then(Value::as_f64)
                        .is_none_or(|min| number >= min)
                    && field
                        .get("maximum")
                        .and_then(Value::as_f64)
                        .is_none_or(|max| number <= max)
            }),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.as_array().is_some_and(|values| {
                let choices = enum_values(&field["items"])
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                field
                    .get("minItems")
                    .and_then(Value::as_u64)
                    .is_none_or(|min| values.len() as u64 >= min)
                    && field
                        .get("maxItems")
                        .and_then(Value::as_u64)
                        .is_none_or(|max| values.len() as u64 <= max)
                    && values
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| choices.contains(&s)))
                    && values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        == values.len()
            }),
            _ => false,
        };
        if !valid {
            return Err("A form value does not match the requested schema".into());
        }
    }
    Ok(())
}
pub fn validate_url(value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid elicitation URL")?;
    if value.len() > 8192
        || !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.chars().any(char::is_control)
    {
        return Err("Unsupported elicitation URL".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_types_bounds_required_fields_and_unknown_properties() {
        let schema = serde_json::json!({"type":"object","properties":{"text":{"type":"string","minLength":2,"maxLength":4},"count":{"type":"integer","minimum":1,"maximum":3},"flag":{"type":"boolean"},"choices":{"type":"array","items":{"type":"string","enum":["a","b"]},"minItems":1,"maxItems":2}},"required":["text","count"]});
        assert!(validate_content(
            &schema,
            &serde_json::json!({"text":"\u{6587}\u{66f8}","count":2,"flag":false,"choices":["a"]})
        )
        .is_ok());
        for content in [
            serde_json::json!({"text":"a","count":2}),
            serde_json::json!({"text":"good","count":1.5}),
            serde_json::json!({"text":"good","count":4}),
            serde_json::json!({"text":"good"}),
            serde_json::json!({"text":"good","count":2,"extra":"bad"}),
            serde_json::json!({"text":"good","count":2,"choices":["a","a"]}),
        ] {
            assert!(validate_content(&schema, &content).is_err());
        }
    }
    #[test]
    fn unsupported_schema_constraints_are_never_silently_ignored() {
        for field in [
            serde_json::json!({"type":"object"}),
            serde_json::json!({"type":"string","pattern":".*"}),
            serde_json::json!({"type":"string","format":"email"}),
            serde_json::json!({"type":"string","enum":["a","a"]}),
        ] {
            assert!(validate_schema(
                &serde_json::json!({"type":"object","properties":{"field":field}})
            )
            .is_err());
        }
        assert!(validate_schema(
            &serde_json::json!({"type":"object","properties":{},"required":["unknown"]})
        )
        .is_err());
    }
    #[test]
    fn urls_require_an_explicit_web_destination_without_userinfo() {
        assert!(validate_url("https://example.com/authorize?state=fixture").is_ok());
        for url in [
            "javascript:alert(1)",
            "file:///private/tmp/test",
            "https://user:secret@example.com/",
            "relative/path",
        ] {
            assert!(validate_url(url).is_err());
        }
    }
}
