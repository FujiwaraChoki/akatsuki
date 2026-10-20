//! A minimal incremental Server-Sent Events parser for provider streams.

/// One SSE event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` field, if any.
    pub event: Option<String>,
    /// `data:` lines joined with `\n`.
    pub data: String,
}

/// Feed bytes in any chunking; complete events come out.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// A new parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n' || *b == b'\r') {
            if self.buf[pos] == b'\r' && pos + 1 == self.buf.len() {
                break; // maybe the first half of \r\n
            }
            let line: Vec<u8> = self.buf.drain(..pos).collect();
            let t = self.buf.remove(0);
            if t == b'\r' && self.buf.first() == Some(&b'\n') {
                self.buf.remove(0);
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if let Some(e) = self.line(&line) {
                out.push(e);
            }
        }
        out
    }

    /// Flushes a final unterminated event.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if !self.buf.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned();
            if let Some(e) = self.line(&rest) {
                return Some(e);
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() {
            self.event = None;
            return None;
        }
        Some(SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data).join("\n") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_at_a_time() {
        let input = b"event: a\r\ndata: 1\r\n\r\n: ping\ndata: {\"x\":\ndata: 2}\n\ndata: tail";
        let mut p = SseParser::new();
        let mut out = Vec::new();
        for b in input {
            out.extend(p.feed(std::slice::from_ref(b)));
        }
        out.extend(p.finish());
        assert_eq!(
            out,
            vec![
                SseEvent { event: Some("a".into()), data: "1".into() },
                SseEvent { event: None, data: "{\"x\":\n2}".into() },
                SseEvent { event: None, data: "tail".into() },
            ]
        );
    }
}
