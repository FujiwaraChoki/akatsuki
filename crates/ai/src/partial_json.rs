//! Best-effort parsing of a JSON prefix into a "deep partial" object, for AASP `partial` events.
//!
//! The prefix is completed by closing open strings, arrays and objects after dropping a trailing
//! incomplete token (a dangling key, `:`, `,`, or a partial `true`/`false`/`null`/number).

use serde_json::Value;

/// Parses the longest meaningful prefix of `text`. Returns `None` before the first `{` or `[`.
pub fn parse_partial(text: &str) -> Option<Value> {
    let start = text.find(['{', '['])?;
    let s = &text[start..];
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    // Scan, tracking nesting and string state, remembering the last position where truncating
    // leaves a well-formed prefix.
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut cut = 0; // byte index up to which the prefix is safe to close
    let mut cut_stack: Vec<char> = Vec::new();
    let mut string_start = 0;
    let mut string_is_key = false;
    let bytes: Vec<(usize, char)> = s.char_indices().collect();
    for &(i, c) in &bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                if !string_is_key {
                    cut = i + 1;
                    cut_stack = stack.clone();
                }
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                string_start = i;
                // A string is a key when it follows `{` or `,` inside an object.
                let prev = s[..i].trim_end().chars().last();
                string_is_key = stack.last() == Some(&'{') && matches!(prev, Some('{') | Some(','));
            }
            '{' | '[' => {
                stack.push(c);
                cut = i + 1;
                cut_stack = stack.clone();
            }
            '}' | ']' => {
                stack.pop();
                cut = i + 1;
                cut_stack = stack.clone();
            }
            ',' | ':' => {}
            c if c.is_whitespace() => {}
            _ => {
                // Inside a literal or number: safe only once the literal is complete, which the
                // next structural character proves; record the position after it optimistically.
                let rest = &s[i..];
                let complete = ["true", "false", "null"].iter().any(|l| rest.starts_with(l))
                    || (c.is_ascii_digit() || c == '-') && rest.find([',', '}', ']']).is_some();
                if complete {
                    let len = rest.find([',', '}', ']']).unwrap_or(rest.len());
                    let candidate = i + len;
                    if candidate > cut && rest[..len].trim().parse::<Value>().is_ok() {
                        cut = candidate;
                        cut_stack = stack.clone();
                    }
                }
            }
        }
    }
    // An unterminated value string: keep what arrived so far.
    let mut prefix = if in_string && !string_is_key {
        let mut p = s.to_owned();
        if escaped {
            p.pop();
        }
        p.push('"');
        cut_stack = stack.clone();
        p
    } else {
        let _ = string_start;
        s[..cut].to_owned()
    };
    // Drop a dangling `,` or `"key":` before closing.
    loop {
        let t = prefix.trim_end();
        if let Some(stripped) = t.strip_suffix(',') {
            prefix = stripped.to_owned();
            continue;
        }
        if let Some(stripped) = t.strip_suffix(':') {
            // remove the key too
            let without = stripped.trim_end();
            if let Some(q) = without.strip_suffix('"')
                && let Some(open) = q.rfind('"')
            {
                prefix = q[..open].to_owned();
                continue;
            }
            prefix = without.to_owned();
            continue;
        }
        break;
    }
    for open in cut_stack.iter().rev() {
        prefix.push(if *open == '{' { '}' } else { ']' });
    }
    serde_json::from_str(&prefix).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prefixes() {
        assert_eq!(parse_partial("Sure: "), None);
        assert_eq!(parse_partial("{"), Some(json!({})));
        assert_eq!(parse_partial("{\"tl"), Some(json!({})));
        assert_eq!(parse_partial("{\"tldr\":"), Some(json!({})));
        assert_eq!(parse_partial("{\"tldr\": \"Lun"), Some(json!({"tldr": "Lun"})));
        assert_eq!(parse_partial("{\"tldr\": \"a\\\""), Some(json!({"tldr": "a\""})));
        assert_eq!(
            parse_partial("{\"tldr\": \"a\", \"bullets\": [\"x\", \"y"),
            Some(json!({"tldr": "a", "bullets": ["x", "y"]}))
        );
        assert_eq!(parse_partial("{\"a\": tr"), Some(json!({})));
        assert_eq!(parse_partial("{\"a\": true, \"n\": 12"), Some(json!({"a": true})));
        assert_eq!(parse_partial("{\"a\": true, \"n\": 12,"), Some(json!({"a": true, "n": 12})));
        assert_eq!(parse_partial("{\"a\": [{\"b\": 1}, {\"c\""), Some(json!({"a": [{"b": 1}, {}]})));
        assert_eq!(parse_partial("```json\n{\"a\": 1}\n```"), None.or(Some(json!({"a": 1}))));
    }

    #[test]
    fn every_prefix_parses_or_is_none() {
        let full = r#"{"replies":[{"label":"Yes","body":"Sounds good — see you at noon."},{"label":"No","body":"Can't, sorry."}],"n":3.5,"ok":false}"#;
        let mut last = json!({});
        for (i, _) in full.char_indices().skip(1) {
            if let Some(v) = parse_partial(&full[..i]) {
                last = v;
            }
        }
        assert_eq!(parse_partial(full).unwrap(), serde_json::from_str::<Value>(full).unwrap());
        assert!(last.get("replies").is_some());
    }
}
