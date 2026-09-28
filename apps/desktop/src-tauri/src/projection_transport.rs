//! Readable JSON at the native Agent transport boundary. Domain identity stays canonical.
use crate::live_sync::CanonicalProjection;
use serde_json::Value;

// An empirical layout target, not a provider requirement or model-token limit.
// Keys and scalar values stay together even when their line exceeds this width.
const SOFT_LINE_BYTES: usize = 384;

pub(crate) fn render(canonical: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(canonical)
        .map_err(|error| format!("unable to decode Agent projection: {error}"))?;
    validate_canonical_identity(&value, canonical)?;
    let text =
        layout(&value).map_err(|error| format!("unable to lay out Agent projection: {error}"))?;
    let decoded: Value = serde_json::from_str(&text)
        .map_err(|error| format!("unable to decode Agent projection transport: {error}"))?;
    validate_canonical_identity(&decoded, canonical)?;
    Ok(text)
}

fn validate_canonical_identity(value: &Value, expected: &str) -> Result<(), String> {
    let actual = CanonicalProjection::from_serializable(value)
        .map_err(|error| format!("unable to validate Agent projection transport: {error}"))?;
    if actual.bytes() != expected.as_bytes() {
        return Err("Agent projection transport changed its canonical content".into());
    }
    Ok(())
}

// Encode each key/scalar once and measure flat subtrees bottom-up. Containers
// retain lengths and child plans, never copied strings for every full subtree.
struct Node {
    flat_len: usize,
    shape: Shape,
}
enum Shape {
    Atom(String),
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
}
impl Node {
    fn from_value(v: &Value) -> serde_json::Result<Self> {
        let shape = match v {
            Value::Array(items) => Shape::Array(
                items
                    .iter()
                    .map(Self::from_value)
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(items) => {
                let mut members: Vec<_> = items.iter().collect();
                // JCS sorts property names by UTF-16 code units, not Rust char order.
                members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                Shape::Object(
                    members
                        .into_iter()
                        .map(|(k, v)| {
                            Ok((
                                serde_json_canonicalizer::to_string(k)?,
                                Self::from_value(v)?,
                            ))
                        })
                        .collect::<serde_json::Result<_>>()?,
                )
            }
            _ => Shape::Atom(serde_json_canonicalizer::to_string(v)?),
        };
        let flat_len = match &shape {
            Shape::Atom(s) => s.len(),
            Shape::Array(a) => {
                2 + a.iter().map(|n| n.flat_len).sum::<usize>() + a.len().saturating_sub(1) * 2
            }
            Shape::Object(m) => {
                2 + m
                    .iter()
                    .map(|(k, n)| k.len() + 2 + n.flat_len)
                    .sum::<usize>()
                    + m.len().saturating_sub(1) * 2
            }
        };
        Ok(Self { flat_len, shape })
    }
    fn flat(&self, out: &mut Writer) {
        match &self.shape {
            Shape::Atom(s) => out.push(s),
            Shape::Array(items) => {
                out.push("[");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(", ");
                    }
                    item.flat(out);
                }
                out.push("]");
            }
            Shape::Object(items) => {
                out.push("{");
                for (i, (key, item)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(", ");
                    }
                    out.push(key);
                    out.push(": ");
                    item.flat(out);
                }
                out.push("}");
            }
        }
    }
    fn render(&self, out: &mut Writer, depth: usize) {
        if self.flat_len <= out.width.saturating_sub(out.column)
            || self.flat_len <= 2
            || matches!(self.shape, Shape::Atom(_))
        {
            self.flat(out);
            return;
        }
        let (open, close) = match &self.shape {
            Shape::Object(_) => ("{", "}"),
            Shape::Array(_) => ("[", "]"),
            _ => unreachable!(),
        };
        out.push(open);
        out.line(depth + 1);
        match &self.shape {
            Shape::Object(items) => {
                for (i, (key, node)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(",");
                        if key.len() + 2 + node.flat_len < out.width.saturating_sub(out.column) {
                            out.push(" ");
                        } else {
                            out.line(depth + 1);
                        }
                    }
                    out.push(key);
                    out.push(": ");
                    node.render(out, depth + 1);
                }
            }
            Shape::Array(items) => {
                for (i, node) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(",");
                        if node.flat_len < out.width.saturating_sub(out.column) {
                            out.push(" ");
                        } else {
                            out.line(depth + 1);
                        }
                    }
                    node.render(out, depth + 1);
                }
            }
            _ => unreachable!(),
        }
        out.line(depth);
        out.push(close);
    }
}
struct Writer {
    text: String,
    column: usize,
    width: usize,
}
impl Writer {
    fn push(&mut self, s: &str) {
        self.text.push_str(s);
        self.column += s.len();
    }
    fn line(&mut self, depth: usize) {
        self.text.push('\n');
        for _ in 0..depth * 2 {
            self.text.push(' ');
        }
        self.column = depth * 2;
    }
}
fn layout(value: &Value) -> serde_json::Result<String> {
    let node = Node::from_value(value)?;
    let mut out = Writer {
        text: String::new(),
        column: 0,
        width: SOFT_LINE_BYTES,
    };
    node.render(&mut out, 0);
    Ok(out.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assert_layout(value: &Value) -> String {
        let canonical = CanonicalProjection::from_serializable(value).unwrap();
        let canonical = std::str::from_utf8(canonical.bytes()).unwrap();
        let rendered = render(canonical).unwrap();
        assert_eq!(rendered, render(canonical).unwrap());
        let decoded: Value = serde_json::from_str(&rendered).unwrap();
        validate_canonical_identity(&decoded, canonical).unwrap();
        // Scan lexical boundaries, ignoring escaped punctuation inside strings.
        let mut quoted = false;
        let mut escaped = false;
        for (index, byte) in rendered.bytes().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
            } else if byte == b'"' {
                quoted = true;
            } else if byte == b':' {
                assert!(!rendered.as_bytes()[index + 1..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_whitespace())
                    .any(|byte| *byte == b'\n'));
            }
        }
        rendered
    }

    #[test]
    fn groups_small_containers_and_splits_large_arrays_between_elements() {
        let records: Vec<_> = (0..20)
            .map(|index| json!({"id":index,"text":"synthetic document text"}))
            .collect();
        let value = json!({"records":records,"small":{"a":1,"b":[2,3]},"empty":[{},[]]});
        let text = assert_layout(&value);
        assert!(text.lines().count() > 2);
        assert!(text.contains("\"small\": {\"a\": 1, \"b\": [2, 3]}"));
        assert!(text.contains("\"empty\": [{}, []]"));
        for index in 0..20 {
            assert!(text.contains(&format!(
                "{{\"id\": {index}, \"text\": \"synthetic document text\"}}"
            )));
        }
    }

    #[test]
    fn splits_large_objects_between_members_and_recurses_into_large_children() {
        let value = json!({
            "outer": {
                "a": "a".repeat(180),
                "b": {"child": "b".repeat(180), "other": "c".repeat(180)},
                "c": ["d".repeat(200), "e".repeat(200)]
            }
        });
        let text = assert_layout(&value);
        assert!(text.contains("\"b\": {\n"));
        assert!(text.contains("\"c\": [\n"));
        assert!(text
            .lines()
            .any(|line| line.trim_start().starts_with("\"other\": ")));
    }

    #[test]
    fn preserves_deep_escaped_unicode_numeric_and_non_object_values() {
        let mut nested = json!({
            "escapes":"\"\\,:[]{}\n\t\u{65e5}\u{672c}\u{8a9e}\u{1f680}",
            "numbers":[-0.0,-1.25e-20,1e30,9007199254740991u64,1e-7,1e-6,1e20,1e21]
        });
        for _ in 0..40 {
            nested = json!({"branch":[nested,{"sibling":true}]});
        }
        assert_layout(&nested);
        for value in [
            Value::Null,
            json!("\"\\\u{1f680}"),
            json!([{}, [], null, -0.0, true, 1e30]),
        ] {
            assert_layout(&value);
        }
        let keys = json!({"\u{1f600}":1,"\u{e000}":2,"\u{10000}":3,"a":4});
        let text = assert_layout(&keys);
        assert!(text.find('\u{10000}').unwrap() < text.find('\u{1f600}').unwrap());
        assert!(text.find('\u{1f600}').unwrap() < text.find('\u{e000}').unwrap());
    }

    #[test]
    fn indivisible_scalar_and_key_can_exceed_the_soft_width() {
        let huge = "x".repeat(120_000);
        let long_key = "k".repeat(600);
        let value = json!({"huge":huge,"tail":{long_key.clone():7},"empty":{}});
        let text = assert_layout(&value);
        assert!(text
            .lines()
            .any(|line| line.contains("\"huge\": \"") && line.len() > 120_000));
        assert!(text.contains(&format!("\"{long_key}\": 7")));
        assert!(text.contains("\"empty\": {}"));
    }

    #[test]
    fn rejects_invalid_or_noncanonical_input_before_rendering() {
        for text in [
            "{",
            "{\"b\":1,\"a\":2}",
            "{ \"a\": 1 }",
            "{\"a\":1,\"a\":2}",
            "-0.0",
        ] {
            assert!(render(text).is_err(), "accepted {text}");
        }
    }
}
