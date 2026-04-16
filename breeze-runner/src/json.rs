/// Minimal JSON encoder for the inbox.json and activity.log contracts.
/// Reads are delegated to `jq` (via the existing executor pattern) so this
/// module stays write-only and tiny. Not a general-purpose serde replacement.
#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn str(value: impl Into<String>) -> Self {
        Json::String(value.into())
    }

    pub fn str_or_null(value: Option<impl Into<String>>) -> Self {
        match value {
            Some(inner) => Json::String(inner.into()),
            None => Json::Null,
        }
    }

    pub fn number_or_null(value: Option<i64>) -> Self {
        match value {
            Some(inner) => Json::Number(inner),
            None => Json::Null,
        }
    }

    pub fn array_of_strings(values: impl IntoIterator<Item = String>) -> Self {
        Json::Array(values.into_iter().map(Json::String).collect())
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        encode_into(&mut output, self);
        output
    }
}

fn encode_into(output: &mut String, value: &Json) {
    match value {
        Json::Null => output.push_str("null"),
        Json::Bool(true) => output.push_str("true"),
        Json::Bool(false) => output.push_str("false"),
        Json::Number(number) => output.push_str(&number.to_string()),
        Json::String(string) => encode_string(output, string),
        Json::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                encode_into(output, item);
            }
            output.push(']');
        }
        Json::Object(entries) => {
            output.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                encode_string(output, key);
                output.push(':');
                encode_into(output, value);
            }
            output.push('}');
        }
    }
}

fn encode_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            code if (code as u32) < 0x20 => {
                output.push_str(&format!("\\u{:04x}", code as u32));
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn encodes_primitives_and_escapes_control_characters() {
        assert_eq!(Json::Null.encode(), "null");
        assert_eq!(Json::Bool(true).encode(), "true");
        assert_eq!(Json::Number(42).encode(), "42");
        assert_eq!(Json::str("hello").encode(), "\"hello\"");
        assert_eq!(Json::str("a\nb\"c").encode(), "\"a\\nb\\\"c\"");
        assert_eq!(Json::str("tab\there").encode(), "\"tab\\there\"");
        assert_eq!(Json::str("\u{0001}").encode(), "\"\\u0001\"");
    }

    #[test]
    fn encodes_arrays_and_objects() {
        let object = Json::Object(vec![
            ("id".to_string(), Json::str("abc")),
            ("count".to_string(), Json::Number(3)),
            (
                "tags".to_string(),
                Json::Array(vec![Json::str("a"), Json::str("b")]),
            ),
        ]);
        assert_eq!(object.encode(), "{\"id\":\"abc\",\"count\":3,\"tags\":[\"a\",\"b\"]}");
    }

    #[test]
    fn encodes_null_arms_of_optional_helpers() {
        assert_eq!(Json::str_or_null(None::<String>).encode(), "null");
        assert_eq!(Json::number_or_null(None).encode(), "null");
        assert_eq!(Json::number_or_null(Some(12)).encode(), "12");
    }

    #[test]
    fn encodes_array_of_strings_helper() {
        assert_eq!(
            Json::array_of_strings(["breeze:wip".to_string(), "breeze:human".to_string()])
                .encode(),
            "[\"breeze:wip\",\"breeze:human\"]"
        );
    }

}
