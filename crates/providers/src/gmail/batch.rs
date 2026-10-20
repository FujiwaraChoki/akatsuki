//! Gmail HTTP batch requests (`POST /batch/gmail/v1`, `multipart/mixed`).
//!
//! Google allows 100 calls per batch and recommends at most 50 (§9.3 uses 50). Each part is a
//! complete HTTP request with a path only; each response part answers the request with the same
//! `Content-ID`, prefixed `response-`, in the same order.

/// One inner request (`GET` only: the adapter batches reads).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchItem {
    /// Path and query, starting with `/gmail/v1/`.
    pub path: String,
}

/// One inner response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchResponse {
    /// HTTP status.
    pub status: u16,
    /// Response headers (lowercased names).
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Vec<u8>,
}

/// Builds the multipart body. Content ids are `item{n}` (1-based).
pub fn encode(items: &[BatchItem], boundary: &str) -> Vec<u8> {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        out.push_str(&format!(
            "--{boundary}\r\nContent-Type: application/http\r\nContent-ID: <item{}>\r\n\r\nGET {} HTTP/1.1\r\n\r\n",
            i + 1,
            item.path
        ));
    }
    out.push_str(&format!("--{boundary}--\r\n"));
    out.into_bytes()
}

/// The `boundary` parameter of a `multipart/*` content type.
pub fn boundary_of(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("boundary").then(|| v.trim().trim_matches('"').to_owned())
    })
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// Splits headers from the rest at the first blank line (`\r\n\r\n` or `\n\n`).
fn split_head(b: &[u8]) -> (&[u8], &[u8]) {
    let crlf = find(b, b"\r\n\r\n", 0).map(|i| (i, 4));
    let lf = find(b, b"\n\n", 0).map(|i| (i, 2));
    let pick = match (crlf, lf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    };
    match pick {
        Some((i, n)) => (&b[..i], &b[i + n..]),
        None => (b, &[]),
    }
}

fn header_lines(head: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(head)
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        })
        .collect()
}

/// Parses a batch response. Parts are returned in request order: by `Content-ID`
/// (`response-item{n}`) when present, else in the order received. `expected` is the number of
/// requests; a missing answer is reported as status 0.
pub fn decode(body: &[u8], boundary: &str, expected: usize) -> Result<Vec<BatchResponse>, String> {
    let delim = format!("--{boundary}");
    let delim = delim.as_bytes();
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut pos = find(body, delim, 0).ok_or("batch response has no boundary")?;
    loop {
        let start = pos + delim.len();
        if body[start..].starts_with(b"--") {
            break;
        }
        let next = find(body, delim, start).ok_or("unterminated batch part")?;
        parts.push(&body[start..next]);
        pos = next;
    }
    let mut ordered: Vec<Option<BatchResponse>> = vec![None; expected];
    let mut unkeyed = Vec::new();
    for part in parts {
        let part = part.strip_prefix(b"\r\n").or_else(|| part.strip_prefix(b"\n")).unwrap_or(part);
        let (outer, inner) = split_head(part);
        let outer = header_lines(outer);
        let (head, payload) = split_head(inner);
        let head_text = String::from_utf8_lossy(head);
        let mut lines = head_text.lines();
        let status_line = lines.next().ok_or("empty batch part")?;
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "bad status line in batch part".to_owned())?;
        let headers = header_lines(head_text.lines().skip(1).collect::<Vec<_>>().join("\n").as_bytes());
        let mut payload = payload.to_vec();
        while payload.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            payload.pop();
        }
        let resp = BatchResponse { status, headers, body: payload };
        let index = outer.iter().find(|(k, _)| k == "content-id").and_then(|(_, v)| {
            let v = v.trim_matches(|c| c == '<' || c == '>');
            v.strip_prefix("response-item")?.parse::<usize>().ok()
        });
        match index {
            Some(n) if n >= 1 && n <= expected && ordered[n - 1].is_none() => ordered[n - 1] = Some(resp),
            _ => unkeyed.push(resp),
        }
    }
    let mut unkeyed = unkeyed.into_iter();
    Ok(ordered
        .into_iter()
        .map(|o| o.or_else(|| unkeyed.next()).unwrap_or(BatchResponse { status: 0, headers: vec![], body: vec![] }))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_parts() {
        let body = encode(
            &[
                BatchItem { path: "/gmail/v1/users/me/threads/a".into() },
                BatchItem { path: "/gmail/v1/users/me/threads/b".into() },
            ],
            "b1",
        );
        let s = String::from_utf8(body).unwrap();
        assert!(s.starts_with("--b1\r\nContent-Type: application/http\r\nContent-ID: <item1>\r\n\r\nGET /gmail/v1/users/me/threads/a HTTP/1.1\r\n"));
        assert!(s.contains("Content-ID: <item2>"));
        assert!(s.ends_with("--b1--\r\n"));
    }

    #[test]
    fn decodes_out_of_order_and_lf_only() {
        let body = "--batch_x\r\nContent-Type: application/http\r\nContent-ID: <response-item2>\r\n\r\nHTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n\r\n{\"error\":{\"code\":404}}\r\n--batch_x\nContent-Type: application/http\nContent-ID: <response-item1>\n\nHTTP/1.1 200 OK\nContent-Type: application/json; charset=UTF-8\n\n{\"id\":\"t1\"}\n--batch_x--\r\n";
        let r = decode(body.as_bytes(), "batch_x", 3).unwrap();
        assert_eq!(r[0].status, 200);
        assert_eq!(r[0].body, b"{\"id\":\"t1\"}");
        assert_eq!(r[0].headers[0], ("content-type".into(), "application/json; charset=UTF-8".into()));
        assert_eq!(r[1].status, 404);
        assert_eq!(r[2].status, 0, "missing answer");
        assert_eq!(boundary_of("multipart/mixed; boundary=\"batch_x\""), Some("batch_x".into()));
        assert!(decode(b"garbage", "batch_x", 1).is_err());
    }
}
