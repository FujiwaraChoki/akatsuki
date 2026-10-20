//! The Akatsuki AI Stream Protocol (AASP, ARCHITECTURE §12.6, D-040).
//!
//! AASP is Server-Sent Events with five event names:
//!
//! ```text
//! event: meta     data: {"feature":"summarize","model":"anthropic/claude-sonnet-5.5","promptVersion":1}
//! event: delta    data: {"text":"…"}                      (text features)
//! event: partial  data: {"object":{…deep partial…}}       (object features)
//! event: done     data: {"text":null,"object":{…},"usage":{"inputTokens":0,"outputTokens":0},"finishReason":"stop"}
//! event: error    data: {"code":"provider_unavailable","message":"…","retryable":true}
//! ```
//!
//! [`AaspEvent::encode`] produces a frame and [`AaspParser`] consumes a byte stream in arbitrary
//! chunks. The TypeScript twin is `packages/wire/src/aasp.ts` (until that package exists, the reader
//! in `packages/ai/src/sse.ts`, D-126); both run `packages/wire/fixtures/aasp/*.txt`.

use serde::{Deserialize, Serialize};

use super::ErrorCode;

/// `meta` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// Feature id.
    pub feature: String,
    /// Resolved model id.
    pub model: String,
    /// Prompt version.
    pub prompt_version: u32,
}

/// Token usage reported in `done`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
}

/// `done` payload. `text` and `object` are always present, `null` when unused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    /// Final text (text features), else `null`.
    pub text: Option<String>,
    /// Final object (object features), else `null`.
    pub object: Option<serde_json::Value>,
    /// Usage.
    pub usage: Usage,
    /// `"stop"`, `"length"`, …
    pub finish_reason: String,
}

/// `error` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamError {
    /// Code from the registry.
    pub code: ErrorCode,
    /// Safe English message.
    pub message: String,
    /// Retryable.
    pub retryable: bool,
    /// Retry delay, when the upstream sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

impl From<ErrorCode> for StreamError {
    fn from(code: ErrorCode) -> Self {
        Self { code, message: code.default_message().to_owned(), retryable: code.retryable(), retry_after_ms: None }
    }
}

/// One AASP event.
#[derive(Clone, Debug, PartialEq)]
pub enum AaspEvent {
    /// Stream metadata, first.
    Meta(Meta),
    /// A text delta.
    Delta {
        /// Appended text.
        text: String,
    },
    /// A deep-partial object snapshot.
    Partial {
        /// The object so far.
        object: serde_json::Value,
    },
    /// Final result, last on success.
    Done(Done),
    /// Error, last on failure.
    Error(StreamError),
}

#[derive(Serialize, Deserialize)]
struct DeltaData {
    text: String,
}

#[derive(Serialize, Deserialize)]
struct PartialData {
    object: serde_json::Value,
}

impl AaspEvent {
    /// The SSE event name.
    pub fn name(&self) -> &'static str {
        match self {
            AaspEvent::Meta(_) => "meta",
            AaspEvent::Delta { .. } => "delta",
            AaspEvent::Partial { .. } => "partial",
            AaspEvent::Done(_) => "done",
            AaspEvent::Error(_) => "error",
        }
    }

    /// The JSON `data` payload, with keys in contract order.
    pub fn data(&self) -> String {
        let v = match self {
            AaspEvent::Meta(m) => serde_json::to_string(m),
            AaspEvent::Delta { text } => serde_json::to_string(&DeltaData { text: text.clone() }),
            AaspEvent::Partial { object } => serde_json::to_string(&PartialData { object: object.clone() }),
            AaspEvent::Done(d) => serde_json::to_string(d),
            AaspEvent::Error(e) => serde_json::to_string(e),
        };
        v.unwrap_or_else(|_| "null".to_owned())
    }

    /// Encodes one SSE frame: `event: <name>\ndata: <json>\n\n`. JSON never contains a raw
    /// newline, so the payload always fits on one `data:` line.
    pub fn encode(&self) -> String {
        format!("event: {}\ndata: {}\n\n", self.name(), self.data())
    }

    /// Decodes an event from its name and data.
    pub fn decode(name: &str, data: &str) -> Result<Self, AaspError> {
        let bad = |e: serde_json::Error| AaspError::BadData { event: name.to_owned(), reason: e.to_string() };
        Ok(match name {
            "meta" => AaspEvent::Meta(serde_json::from_str(data).map_err(bad)?),
            "delta" => AaspEvent::Delta { text: serde_json::from_str::<DeltaData>(data).map_err(bad)?.text },
            "partial" => AaspEvent::Partial { object: serde_json::from_str::<PartialData>(data).map_err(bad)?.object },
            "done" => AaspEvent::Done(serde_json::from_str(data).map_err(bad)?),
            "error" => AaspEvent::Error(serde_json::from_str(data).map_err(bad)?),
            other => return Err(AaspError::UnknownEvent(other.to_owned())),
        })
    }

    /// `true` for `done` and `error`, after which the stream ends.
    pub fn is_terminal(&self) -> bool {
        matches!(self, AaspEvent::Done(_) | AaspEvent::Error(_))
    }
}

/// Errors from [`AaspParser`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AaspError {
    /// An event name outside the protocol.
    #[error("unknown AASP event {0:?}")]
    UnknownEvent(String),
    /// The `data` JSON does not match the event.
    #[error("bad data for AASP event {event:?}: {reason}")]
    BadData {
        /// Event name.
        event: String,
        /// Parser message.
        reason: String,
    },
    /// The byte stream is not UTF-8.
    #[error("AASP stream is not valid UTF-8")]
    Utf8,
}

/// An incremental SSE parser for AASP. Feed it chunks of any size; it returns the complete events
/// found so far. Handles `\n`, `\r\n` and `\r` line endings, comment lines (`:`), multi-line `data`
/// (joined with `\n`) and a missing `event:` line (treated as the SSE default `message`, which is
/// rejected as unknown).
#[derive(Debug, Default)]
pub struct AaspParser {
    pending: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl AaspParser {
    /// A new parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns every event completed by it.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Result<AaspEvent, AaspError>> {
        self.pending.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n' || *b == b'\r') {
            // A lone '\r' at the end of the buffer may be the first half of "\r\n": wait for more.
            if self.pending[pos] == b'\r' && pos + 1 == self.pending.len() {
                break;
            }
            let line: Vec<u8> = self.pending.drain(..pos).collect();
            let terminator = self.pending.remove(0);
            if terminator == b'\r' && self.pending.first() == Some(&b'\n') {
                self.pending.remove(0);
            }
            match String::from_utf8(line) {
                Ok(line) => {
                    if let Some(result) = self.line(&line) {
                        out.push(result);
                    }
                }
                Err(_) => out.push(Err(AaspError::Utf8)),
            }
        }
        out
    }

    /// Signals the end of the stream and returns a final event if one was not terminated by a
    /// blank line.
    pub fn finish(&mut self) -> Option<Result<AaspEvent, AaspError>> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            match String::from_utf8(rest) {
                Ok(line) => {
                    if let Some(r) = self.line(&line) {
                        return Some(r);
                    }
                }
                Err(_) => return Some(Err(AaspError::Utf8)),
            }
        }
        self.dispatch()
    }

    fn line(&mut self, line: &str) -> Option<Result<AaspEvent, AaspError>> {
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

    fn dispatch(&mut self) -> Option<Result<AaspEvent, AaspError>> {
        if self.data.is_empty() {
            self.event = None;
            return None;
        }
        let name = self.event.take().unwrap_or_else(|| "message".to_owned());
        let data = std::mem::take(&mut self.data).join("\n");
        Some(AaspEvent::decode(&name, &data))
    }
}

/// Parses a complete AASP document.
pub fn parse_all(input: &str) -> Vec<Result<AaspEvent, AaspError>> {
    let mut p = AaspParser::new();
    let mut out = p.feed(input.as_bytes());
    out.extend(p.finish());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Vec<AaspEvent> {
        vec![
            AaspEvent::Meta(Meta {
                feature: "summarize".into(),
                model: "anthropic/claude-sonnet-5.5".into(),
                prompt_version: 1,
            }),
            AaspEvent::Partial { object: json!({"tldr": "Lunch"}) },
            AaspEvent::Delta { text: "line\nbreak ünïcode".into() },
            AaspEvent::Done(Done {
                text: None,
                object: Some(json!({"tldr": "Lunch moved"})),
                usage: Usage { input_tokens: 10, output_tokens: 3 },
                finish_reason: "stop".into(),
            }),
        ]
    }

    #[test]
    fn encode_then_parse_round_trips() {
        let events = sample();
        let doc: String = events.iter().map(AaspEvent::encode).collect();
        let parsed: Vec<AaspEvent> = parse_all(&doc).into_iter().map(Result::unwrap).collect();
        assert_eq!(parsed, events);
    }

    #[test]
    fn byte_at_a_time_and_crlf() {
        let doc: String = sample().iter().map(AaspEvent::encode).collect::<String>().replace('\n', "\r\n");
        let mut p = AaspParser::new();
        let mut got = Vec::new();
        for b in doc.as_bytes() {
            got.extend(p.feed(std::slice::from_ref(b)));
        }
        got.extend(p.finish());
        let got: Vec<AaspEvent> = got.into_iter().map(Result::unwrap).collect();
        assert_eq!(got, sample());
    }

    #[test]
    fn done_wire_shape_keeps_nulls() {
        let done = AaspEvent::Done(Done {
            text: Some("hi".into()),
            object: None,
            usage: Usage::default(),
            finish_reason: "stop".into(),
        });
        assert_eq!(
            done.encode(),
            "event: done\ndata: {\"text\":\"hi\",\"object\":null,\"usage\":{\"inputTokens\":0,\"outputTokens\":0},\"finishReason\":\"stop\"}\n\n"
        );
    }

    #[test]
    fn comments_multiline_data_and_errors() {
        let doc = ": keep-alive\nevent: error\ndata: {\"code\":\"provider_unavailable\",\ndata: \"message\":\"down\",\"retryable\":true}\n\nevent: nope\ndata: {}\n\n";
        let out = parse_all(doc);
        assert_eq!(
            out[0],
            Ok(AaspEvent::Error(StreamError {
                code: ErrorCode::ProviderUnavailable,
                message: "down".into(),
                retryable: true,
                retry_after_ms: None,
            }))
        );
        assert_eq!(out[1], Err(AaspError::UnknownEvent("nope".into())));
    }

    #[test]
    fn unterminated_final_event_is_flushed() {
        let out = parse_all("event: delta\ndata: {\"text\":\"x\"}");
        assert_eq!(out, vec![Ok(AaspEvent::Delta { text: "x".into() })]);
    }
}
