//! The streaming client (ARCHITECTURE §12.3–§12.6).
//!
//! - [`Provider::OpenAiCompatible`]: `POST {base}/chat/completions` with `stream: true`. Covers
//!   OpenAI, OpenRouter, Google's and AI Gateway's OpenAI-compatible endpoints, Ollama (`/v1`),
//!   LM Studio, llama.cpp and vLLM. Structured output uses
//!   `response_format: {type: "json_schema", json_schema: {name, strict: true, schema}}`.
//!   OpenRouter requests also carry [`OPENROUTER_PRIVACY`] as `provider` (zero data retention, data
//!   collection denied) and the effort hint as `reasoning: {effort, exclude: true}`, exactly like
//!   `packages/ai` (D-121), so desktop routes the same way as web and mobile.
//! - [`Provider::Anthropic`]: `POST {base}/v1/messages` with `stream: true`. Structured output
//!   uses `output_config.format = {type: "json_schema", schema}` (never a forced tool, which the
//!   5.5 models reject); the effort hint goes to `output_config.effort`.
//! - [`Provider::Akatsuki`]: the managed path, `POST {api}/v1/ai/{feature}` with the user's JWT;
//!   apps/api already speaks AASP, so events are passed through.
//!
//! No request carries sampling parameters (`temperature`): the 5.5 Claude models reject them.
//! Every path produces the same AASP events ([`AaspEvent`]), so UI code is identical whichever
//! provider runs. Object results are validated against the output schema, with one repair retry.

use std::time::Duration;

use akatsuki_core::wire::aasp::{AaspEvent, AaspParser, Done, Meta, Usage};
use futures::stream::{BoxStream, StreamExt};
use serde_json::{Value, json};

use crate::AiError;
use crate::partial_json::parse_partial;
use crate::prompts::{Mode, RenderedPrompt, validate};
use crate::settings::{AiProfile, AiProviderKind, Resolved, base_url};
use crate::sse::SseParser;

/// The Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// OpenRouter provider routing sent with every OpenRouter request (BYO and managed): only
/// zero-data-retention endpoints, never providers that store or train on data. Mirrors
/// `OPENROUTER_PRIVACY` in `packages/ai/src/providers.ts`; both are checked against
/// `packages/ai/fixtures/openrouter/request.json`.
pub const OPENROUTER_PRIVACY: OpenRouterPrivacy =
    OpenRouterPrivacy { zdr: true, data_collection: "deny", allow_fallbacks: true, require_parameters: true };

/// OpenRouter's `provider` request object (the fields Akatsuki sets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct OpenRouterPrivacy {
    /// Only zero-data-retention endpoints.
    pub zdr: bool,
    /// `"deny"`: skip providers that may store or train on prompts.
    pub data_collection: &'static str,
    /// Other ZDR endpoints may serve the model when one is down.
    pub allow_fallbacks: bool,
    /// Only endpoints that support every parameter sent (for example `response_format`).
    pub require_parameters: bool,
}

/// Where a request goes.
#[derive(Clone, PartialEq, Eq)]
pub enum Provider {
    /// Any OpenAI-compatible chat-completions endpoint.
    OpenAiCompatible {
        /// Base URL including the version segment, e.g. `https://api.openai.com/v1`.
        base_url: String,
        /// Bearer key; `None` for local servers.
        api_key: Option<String>,
        /// Extra headers (OpenRouter attribution).
        extra_headers: Vec<(String, String)>,
        /// OpenRouter: adds [`OPENROUTER_PRIVACY`] and the `reasoning` effort to every body.
        openrouter: bool,
    },
    /// Anthropic's Messages API.
    Anthropic {
        /// Base URL without `/v1`, e.g. `https://api.anthropic.com`.
        base_url: String,
        /// `x-api-key`.
        api_key: String,
    },
    /// Akatsuki-managed AI through apps/api.
    Akatsuki {
        /// apps/api base URL, e.g. `https://api.akatsuki.vc`.
        base_url: String,
        /// The user's Convex JWT.
        jwt: String,
    },
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::OpenAiCompatible { base_url, .. } => {
                f.debug_struct("OpenAiCompatible").field("base_url", base_url).finish_non_exhaustive()
            }
            Provider::Anthropic { base_url, .. } => {
                f.debug_struct("Anthropic").field("base_url", base_url).finish_non_exhaustive()
            }
            Provider::Akatsuki { base_url, .. } => {
                f.debug_struct("Akatsuki").field("base_url", base_url).finish_non_exhaustive()
            }
        }
    }
}

impl Provider {
    /// Builds the provider for a profile. `key` comes from the device keychain (service
    /// `vc.akatsuki.ai`, account `<profileId>`) or, for `akatsuki`, is the user's JWT.
    ///
    /// The managed provider never uses `profile.base_url`: the profile is synced user data, and it
    /// would receive the user's session JWT. It goes to the default apps/api origin; use
    /// [`AiClient::provider_for`] to apply this deployment's configured origin (D-415).
    pub fn for_profile(profile: &AiProfile, key: Option<String>) -> Result<Self, AiError> {
        let key = key.filter(|k| !k.trim().is_empty());
        let base = || base_url(profile).map(|u| u.as_str().trim_end_matches('/').to_owned());
        Ok(match profile.kind {
            AiProviderKind::Anthropic => {
                Provider::Anthropic { base_url: base()?, api_key: key.ok_or(AiError::KeyMissing)? }
            }
            AiProviderKind::Akatsuki => {
                Provider::Akatsuki { base_url: DEFAULT_MANAGED_API.to_owned(), jwt: key.ok_or(AiError::KeyMissing)? }
            }
            AiProviderKind::Ollama | AiProviderKind::OpenaiCompatible => Provider::OpenAiCompatible {
                base_url: base()?,
                api_key: key,
                extra_headers: Vec::new(),
                openrouter: false,
            },
            AiProviderKind::Openrouter => Provider::OpenAiCompatible {
                base_url: base()?,
                api_key: Some(key.ok_or(AiError::KeyMissing)?),
                extra_headers: vec![
                    ("HTTP-Referer".into(), "https://akatsuki.vc".into()),
                    ("X-Title".into(), "Akatsuki".into()),
                ],
                openrouter: true,
            },
            AiProviderKind::Openai | AiProviderKind::Google | AiProviderKind::Gateway => Provider::OpenAiCompatible {
                base_url: base()?,
                api_key: Some(key.ok_or(AiError::KeyMissing)?),
                extra_headers: Vec::new(),
                openrouter: false,
            },
        })
    }
}

/// A raw piece of a provider stream.
#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Text(String),
    Usage { input: Option<u64>, output: Option<u64> },
    Finish(String),
}

/// One provider request.
#[derive(Clone, Debug, PartialEq)]
struct ChatRequest {
    model: String,
    system: String,
    messages: Vec<(String, String)>,
    max_tokens: u32,
    schema: Option<(String, Value)>,
    effort: Option<String>,
}

/// The AI client. Cheap to clone.
#[derive(Clone, Debug)]
pub struct AiClient {
    http: reqwest::Client,
    /// apps/api origin for the managed provider (`AKATSUKI_API_URL`), from local configuration only.
    managed_api: Option<String>,
}

/// The managed provider's apps/api origin when none is configured.
pub const DEFAULT_MANAGED_API: &str = "https://api.akatsuki.vc";

impl Default for AiClient {
    fn default() -> Self {
        Self::new()
    }
}

fn retry_after(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| s * 1000)
}

fn map_transport(e: &reqwest::Error) -> AiError {
    let _ = e; // never surface transport text: it may contain the URL
    AiError::Provider {
        code: akatsuki_core::wire::ErrorCode::ProviderUnavailable,
        message: "The AI provider could not be reached.".into(),
        retry_after_ms: None,
    }
}

impl AiClient {
    /// A client with sane timeouts (30 s to connect, no total timeout: streams can be long).
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .user_agent(concat!("akatsuki-desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self { http, managed_api: None }
    }

    /// Wraps an existing `reqwest` client.
    pub fn with_http(http: reqwest::Client) -> Self {
        Self { http, managed_api: None }
    }

    /// Sends managed-AI requests (and the user's JWT) to this apps/api origin instead of
    /// [`DEFAULT_MANAGED_API`]. It comes from the device's own configuration, never from synced
    /// settings (D-415). A value that is not an `https` URL (or `http` on a local address) is
    /// ignored.
    #[must_use]
    pub fn with_managed_api(mut self, origin: Option<String>) -> Self {
        self.managed_api = origin
            .map(|o| o.trim().trim_end_matches('/').to_owned())
            .filter(|o| !o.is_empty() && crate::settings::base_url_problem(o).is_none());
        self
    }

    /// The apps/api origin the managed provider uses.
    pub fn managed_api(&self) -> &str {
        self.managed_api.as_deref().unwrap_or(DEFAULT_MANAGED_API)
    }

    /// [`Provider::for_profile`] with this client's managed apps/api origin.
    pub fn provider_for(&self, profile: &AiProfile, key: Option<String>) -> Result<Provider, AiError> {
        Ok(match Provider::for_profile(profile, key)? {
            Provider::Akatsuki { jwt, .. } => Provider::Akatsuki { base_url: self.managed_api().to_owned(), jwt },
            p => p,
        })
    }

    /// The underlying HTTP client.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    async fn post(&self, provider: &Provider, body: &Value) -> Result<reqwest::Response, AiError> {
        let req = match provider {
            Provider::OpenAiCompatible { base_url, api_key, extra_headers, .. } => {
                let mut r = self.http.post(format!("{base_url}/chat/completions")).json(body);
                if let Some(k) = api_key {
                    r = r.bearer_auth(k);
                }
                for (k, v) in extra_headers {
                    r = r.header(k, v);
                }
                r
            }
            Provider::Anthropic { base_url, api_key } => self
                .http
                .post(format!("{base_url}/v1/messages"))
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(body),
            Provider::Akatsuki { .. } => return Err(AiError::Stream),
        };
        let resp = req.header("accept", "text/event-stream").send().await.map_err(|e| map_transport(&e))?;
        if !resp.status().is_success() {
            return Err(AiError::from_status(resp.status().as_u16(), retry_after(&resp)));
        }
        Ok(resp)
    }

    fn body(provider: &Provider, req: &ChatRequest) -> Value {
        match provider {
            Provider::Anthropic { .. } => {
                let messages: Vec<Value> =
                    req.messages.iter().map(|(role, content)| json!({"role": role, "content": content})).collect();
                let mut body = json!({
                    "model": req.model,
                    "max_tokens": req.max_tokens,
                    "system": req.system,
                    "messages": messages,
                    "stream": true,
                });
                let mut output_config = serde_json::Map::new();
                if let Some((_, schema)) = &req.schema {
                    output_config.insert("format".into(), json!({"type": "json_schema", "schema": schema}));
                }
                if let Some(effort) = &req.effort {
                    output_config.insert("effort".into(), json!(effort));
                }
                if !output_config.is_empty() {
                    body["output_config"] = Value::Object(output_config);
                }
                body
            }
            _ => {
                let mut messages = vec![json!({"role": "system", "content": req.system})];
                messages.extend(req.messages.iter().map(|(role, content)| json!({"role": role, "content": content})));
                let mut body = json!({
                    "model": req.model,
                    "messages": messages,
                    "max_tokens": req.max_tokens,
                    "stream": true,
                    "stream_options": {"include_usage": true},
                });
                if let Some((name, schema)) = &req.schema {
                    body["response_format"] =
                        json!({"type": "json_schema", "json_schema": {"name": name, "strict": true, "schema": schema}});
                }
                if let Provider::OpenAiCompatible { openrouter: true, .. } = provider {
                    body["provider"] = json!(OPENROUTER_PRIVACY);
                    if let Some(effort) = &req.effort {
                        body["reasoning"] = json!({"effort": effort, "exclude": true});
                    }
                }
                body
            }
        }
    }

    /// Streams raw pieces from a provider.
    async fn pieces(
        &self,
        provider: &Provider,
        req: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<Piece, AiError>>, AiError> {
        let body = Self::body(provider, req);
        let resp = self.post(provider, &body).await?;
        let anthropic = matches!(provider, Provider::Anthropic { .. });
        let stream = async_stream::stream! {
            let mut parser = SseParser::new();
            let mut bytes = resp.bytes_stream();
            let mut done = false;
            while let Some(chunk) = bytes.next().await {
                let Ok(chunk) = chunk else {
                    yield Err(AiError::Stream);
                    return;
                };
                for ev in parser.feed(&chunk) {
                    for piece in decode_event(anthropic, &ev.event, &ev.data) {
                        if piece.is_err() { done = true; }
                        yield piece;
                    }
                    if done { return; }
                }
            }
            if let Some(ev) = parser.finish() {
                for piece in decode_event(anthropic, &ev.event, &ev.data) {
                    yield piece;
                }
            }
        };
        Ok(stream.boxed())
    }

    /// Runs a rendered prompt against a resolved profile and yields AASP events. The stream
    /// always ends with exactly one `done` or `error`.
    pub fn run(
        &self,
        provider: Provider,
        resolved: &Resolved<'_>,
        prompt: &RenderedPrompt,
    ) -> BoxStream<'static, AaspEvent> {
        self.run_with(provider, resolved, prompt, None)
    }

    /// [`AiClient::run`] with escalation (D-120): when `escalate_to` is set, the one repair retry
    /// after a schema-invalid answer goes to that heavier model, announced by a second `meta`
    /// event (as `streamFeature` does in `packages/ai`). See [`crate::settings::Escalation`].
    pub fn run_with(
        &self,
        provider: Provider,
        resolved: &Resolved<'_>,
        prompt: &RenderedPrompt,
        escalate_to: Option<String>,
    ) -> BoxStream<'static, AaspEvent> {
        let client = self.clone();
        let feature = prompt.feature.as_str().to_owned();
        let model = resolved.model.clone();
        let prompt = prompt.clone();
        let stream = async_stream::stream! {
            if matches!(provider, Provider::Akatsuki { .. }) {
                // Managed profiles never send a rendered prompt; use `run_managed` with the vars.
                yield AaspEvent::Error(AiError::FeatureOff.to_stream_error());
                return;
            }
            yield AaspEvent::Meta(Meta { feature: feature.clone(), model: model.clone(), prompt_version: prompt.meta.version });
            let schema = prompt.output_schema.clone().filter(|_| prompt.meta.mode == Mode::Object);
            let mut req = ChatRequest {
                model: model.clone(),
                system: prompt.system.clone(),
                messages: vec![("user".into(), prompt.user.clone())],
                max_tokens: prompt.meta.max_output_tokens,
                schema: schema.clone().map(|s| (feature.clone(), s)),
                effort: prompt.meta.effort.map(|e| e.as_str().to_owned()),
            };
            let mut usage = Usage::default();
            let mut attempts = 0;
            loop {
                attempts += 1;
                let mut text = String::new();
                let mut finish = "stop".to_owned();
                let mut last_partial: Option<Value> = None;
                let mut pieces = match client.pieces(&provider, &req).await {
                    Ok(p) => p,
                    Err(e) => {
                        yield AaspEvent::Error(e.to_stream_error());
                        return;
                    }
                };
                while let Some(piece) = pieces.next().await {
                    match piece {
                        Ok(Piece::Text(t)) => {
                            text.push_str(&t);
                            if schema.is_none() {
                                yield AaspEvent::Delta { text: t };
                            } else if attempts == 1
                                && let Some(p) = parse_partial(&text)
                                && last_partial.as_ref() != Some(&p)
                            {
                                last_partial = Some(p.clone());
                                yield AaspEvent::Partial { object: p };
                            }
                        }
                        Ok(Piece::Usage { input, output }) => {
                            if let Some(i) = input { usage.input_tokens += i; }
                            if let Some(o) = output { usage.output_tokens += o; }
                        }
                        Ok(Piece::Finish(f)) => finish = f,
                        Err(e) => {
                            yield AaspEvent::Error(e.to_stream_error());
                            return;
                        }
                    }
                }
                let Some(schema) = &schema else {
                    yield AaspEvent::Done(Done { text: Some(text), object: None, usage, finish_reason: finish });
                    return;
                };
                let parsed = extract_json(&text);
                let problem = match &parsed {
                    Some(v) => validate(schema, v).err(),
                    None => Some("the reply was not JSON".to_owned()),
                };
                match (parsed, problem) {
                    (Some(object), None) => {
                        yield AaspEvent::Done(Done { text: None, object: Some(object), usage, finish_reason: finish });
                        return;
                    }
                    (_, Some(problem)) if attempts == 1 => {
                        // One repair retry: show the model its output and the schema errors, on the
                        // heavier model when the caller escalates.
                        req.messages.push(("assistant".into(), text.clone()));
                        req.messages.push((
                            "user".into(),
                            format!("That reply did not match the required JSON schema ({problem}). Reply again with only the corrected JSON object."),
                        ));
                        if let Some(heavier) = escalate_to.clone().filter(|m| *m != req.model) {
                            req.model = heavier.clone();
                            yield AaspEvent::Meta(Meta { feature: feature.clone(), model: heavier, prompt_version: prompt.meta.version });
                        }
                    }
                    _ => {
                        yield AaspEvent::Error(AiError::InvalidOutput.to_stream_error());
                        return;
                    }
                }
            }
        };
        stream.boxed()
    }

    /// Runs a feature through apps/api (`POST {base}/v1/ai/{feature}`), passing AASP through.
    pub fn run_managed(
        &self,
        base_url: &str,
        jwt: &str,
        feature: &str,
        prompt_version: u32,
        vars: serde_json::Map<String, Value>,
    ) -> BoxStream<'static, AaspEvent> {
        let req = self
            .http
            .post(format!("{}/v1/ai/{feature}", base_url.trim_end_matches('/')))
            .bearer_auth(jwt)
            .header("accept", "text/event-stream")
            .json(&json!({"promptVersion": prompt_version, "vars": vars}));
        let stream = async_stream::stream! {
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    yield AaspEvent::Error(map_transport(&e).to_stream_error());
                    return;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let ra = retry_after(&resp);
                // apps/api errors use the shared envelope; prefer its code.
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let code = body.pointer("/error/code").and_then(Value::as_str).and_then(akatsuki_core::wire::ErrorCode::from_name);
                let mut err = AiError::from_status(status, ra);
                if let (Some(code), AiError::Provider { code: c, message, .. }) = (code, &mut err) {
                    *c = code;
                    *message = code.default_message().to_owned();
                }
                yield AaspEvent::Error(err.to_stream_error());
                return;
            }
            let mut parser = AaspParser::new();
            let mut bytes = resp.bytes_stream();
            while let Some(chunk) = bytes.next().await {
                let Ok(chunk) = chunk else {
                    yield AaspEvent::Error(AiError::Stream.to_stream_error());
                    return;
                };
                for ev in parser.feed(&chunk) {
                    match ev {
                        Ok(ev) => {
                            let terminal = ev.is_terminal();
                            yield ev;
                            if terminal { return; }
                        }
                        Err(_) => {
                            yield AaspEvent::Error(AiError::Stream.to_stream_error());
                            return;
                        }
                    }
                }
            }
            match parser.finish() {
                Some(Ok(ev)) if ev.is_terminal() => yield ev,
                _ => yield AaspEvent::Error(AiError::Stream.to_stream_error()),
            }
        };
        stream.boxed()
    }

    /// "Test connection" (§12.2): Ollama `GET /api/tags`; others a 1-token request, sent from the
    /// device.
    pub async fn test_connection(&self, profile: &AiProfile, provider: &Provider, model: &str) -> Result<(), AiError> {
        if profile.kind == AiProviderKind::Ollama
            && let Provider::OpenAiCompatible { base_url, .. } = provider
        {
            let origin = base_url.trim_end_matches("/v1");
            let resp = self.http.get(format!("{origin}/api/tags")).send().await.map_err(|e| map_transport(&e))?;
            return if resp.status().is_success() {
                Ok(())
            } else {
                Err(AiError::from_status(resp.status().as_u16(), None))
            };
        }
        let req = ChatRequest {
            model: model.to_owned(),
            system: "Reply with one word.".into(),
            messages: vec![("user".into(), "ping".into())],
            max_tokens: 1,
            schema: None,
            effort: None,
        };
        let mut pieces = self.pieces(provider, &req).await?;
        while let Some(p) = pieces.next().await {
            p?;
        }
        Ok(())
    }
}

/// Pulls the JSON object out of a reply (tolerates a Markdown fence or leading prose).
fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    let start = t.find(['{', '['])?;
    let end = t.rfind(['}', ']'])?;
    (end > start).then(|| serde_json::from_str(&t[start..=end]).ok()).flatten()
}

fn decode_event(anthropic: bool, event: &Option<String>, data: &str) -> Vec<Result<Piece, AiError>> {
    if anthropic { decode_anthropic(event.as_deref(), data) } else { decode_openai(data) }
}

fn decode_openai(data: &str) -> Vec<Result<Piece, AiError>> {
    let data = data.trim();
    if data == "[DONE]" || data.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else { return vec![Err(AiError::Stream)] };
    if v.get("error").is_some() {
        let status =
            v.pointer("/error/code").and_then(Value::as_u64).and_then(|c| u16::try_from(c).ok()).unwrap_or(502);
        return vec![Err(AiError::from_status(status, None))];
    }
    let mut out = Vec::new();
    if let Some(choice) = v.pointer("/choices/0") {
        if let Some(t) = choice.pointer("/delta/content").and_then(Value::as_str)
            && !t.is_empty()
        {
            out.push(Ok(Piece::Text(t.to_owned())));
        }
        if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
            out.push(Ok(Piece::Finish(if f == "length" { "length".into() } else { "stop".into() })));
        }
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
        out.push(Ok(Piece::Usage {
            input: u.get("prompt_tokens").and_then(Value::as_u64),
            output: u.get("completion_tokens").and_then(Value::as_u64),
        }));
    }
    out
}

fn decode_anthropic(event: Option<&str>, data: &str) -> Vec<Result<Piece, AiError>> {
    let Ok(v) = serde_json::from_str::<Value>(data) else { return vec![Err(AiError::Stream)] };
    let kind = event.or_else(|| v.get("type").and_then(Value::as_str)).unwrap_or("");
    match kind {
        "message_start" => {
            // `output_tokens` here is a starting value; `message_delta` carries the final count.
            let u = v.pointer("/message/usage");
            vec![Ok(Piece::Usage {
                input: u.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64),
                output: None,
            })]
        }
        "content_block_delta" => match v.pointer("/delta/type").and_then(Value::as_str) {
            // Thinking and other deltas are never shown.
            Some("text_delta") => v
                .pointer("/delta/text")
                .and_then(Value::as_str)
                .map(|t| vec![Ok(Piece::Text(t.to_owned()))])
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        "message_delta" => {
            let mut out = Vec::new();
            if let Some(o) = v.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                out.push(Ok(Piece::Usage { input: None, output: Some(o) }));
            }
            if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                let reason = match r {
                    "max_tokens" => "length",
                    "refusal" => "refusal",
                    _ => "stop",
                };
                out.push(Ok(Piece::Finish(reason.into())));
            }
            out
        }
        "error" => {
            let status = match v.pointer("/error/type").and_then(Value::as_str) {
                Some("overloaded_error") => 529,
                Some("rate_limit_error") => 429,
                Some("authentication_error") => 401,
                Some("invalid_request_error") => 400,
                _ => 500,
            };
            vec![Err(AiError::from_status(status, None))]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: AiProviderKind) -> Provider {
        let profile = AiProfile {
            profile_id: "p".into(),
            label: "P".into(),
            kind,
            base_url: None,
            models: Default::default(),
            key_on_devices: vec![],
        };
        Provider::for_profile(&profile, Some("k".into())).unwrap()
    }

    fn request(effort: Option<&str>) -> ChatRequest {
        ChatRequest {
            model: "anthropic/claude-sonnet-5.5".into(),
            system: "s".into(),
            messages: vec![("user".into(), "u".into())],
            max_tokens: 10,
            schema: None,
            effort: effort.map(str::to_owned),
        }
    }

    #[test]
    fn openrouter_bodies_match_the_shared_fixture() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/ai/fixtures/openrouter/request.json");
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json!(OPENROUTER_PRIVACY), fx["provider"]);
        let or = provider(AiProviderKind::Openrouter);
        for case in fx["cases"].as_array().unwrap() {
            let body = AiClient::body(&or, &request(case["effort"].as_str()));
            assert_eq!(body["provider"], fx["provider"], "{case}");
            assert_eq!(body.get("reasoning").cloned().unwrap_or(Value::Null), case["reasoning"], "{case}");
        }
        // Only OpenRouter gets OpenRouter's routing object.
        for kind in [AiProviderKind::Openai, AiProviderKind::Gateway, AiProviderKind::Ollama, AiProviderKind::Google] {
            let body = AiClient::body(&provider(kind), &request(Some("low")));
            assert!(body.get("provider").is_none() && body.get("reasoning").is_none(), "{kind:?}");
        }
    }

    #[test]
    fn openai_chunks() {
        assert_eq!(
            decode_openai(r#"{"choices":[{"delta":{"content":"Hi"},"finish_reason":null}]}"#),
            vec![Ok(Piece::Text("Hi".into()))]
        );
        assert_eq!(
            decode_openai(r#"{"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#),
            vec![Ok(Piece::Usage { input: Some(5), output: Some(2) })]
        );
        assert!(decode_openai("[DONE]").is_empty());
        assert!(decode_openai("not json")[0].is_err());
    }

    #[test]
    fn anthropic_events() {
        assert_eq!(
            decode_anthropic(
                Some("content_block_delta"),
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"secret"}}"#
            ),
            vec![]
        );
        assert_eq!(
            decode_anthropic(
                None,
                r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":9}}"#
            ),
            vec![Ok(Piece::Usage { input: None, output: Some(9) }), Ok(Piece::Finish("length".into()))]
        );
        assert!(
            decode_anthropic(Some("error"), r#"{"type":"error","error":{"type":"overloaded_error","message":"x"}}"#)[0]
                .is_err()
        );
    }

    #[test]
    fn json_extraction() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```"), Some(json!({"a": 1})));
        assert_eq!(extract_json("nope"), None);
    }
}
