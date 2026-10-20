//! The transport under the typed APIs: the official `convex` client in production, a scripted
//! mock in tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::Value as Json;

use crate::ConvexApiError;
use crate::value::{convex_to_json, json_to_convex};

/// A live query result stream.
pub type JsonStream = BoxStream<'static, Result<Json, ConvexApiError>>;

/// Raw Convex calls with JSON arguments and results.
#[async_trait]
pub trait ConvexTransport: Send + Sync {
    /// Runs a query once.
    async fn query(&self, function: &str, args: Json) -> Result<Json, ConvexApiError>;
    /// Runs a mutation.
    async fn mutation(&self, function: &str, args: Json) -> Result<Json, ConvexApiError>;
    /// Runs an action.
    async fn action(&self, function: &str, args: Json) -> Result<Json, ConvexApiError>;
    /// Subscribes to a query; every new result arrives on the stream.
    async fn subscribe(&self, function: &str, args: Json) -> Result<JsonStream, ConvexApiError>;
    /// Sets (or clears) the JWT sent with every call.
    async fn set_auth(&self, token: Option<String>);
}

fn args_map(args: Json) -> Result<std::collections::BTreeMap<String, convex::Value>, ConvexApiError> {
    match args {
        Json::Object(m) => m.into_iter().map(|(k, v)| Ok((k, json_to_convex(v)?))).collect(),
        Json::Null => Ok(Default::default()),
        _ => Err(ConvexApiError::Encode("Convex arguments must be an object".into())),
    }
}

fn result(function: &str, r: convex::FunctionResult) -> Result<Json, ConvexApiError> {
    match r {
        convex::FunctionResult::Value(v) => Ok(convex_to_json(v)),
        convex::FunctionResult::ErrorMessage(message) => {
            Err(ConvexApiError::Function { function: function.to_owned(), message })
        }
        convex::FunctionResult::ConvexError(e) => {
            Err(ConvexApiError::from_app_error(function, e.message, convex_to_json(e.data)))
        }
    }
}

/// The production transport: one shared `convex::ConvexClient` connection.
#[derive(Clone)]
pub struct ConvexClientTransport {
    client: convex::ConvexClient,
}

impl std::fmt::Debug for ConvexClientTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvexClientTransport").finish_non_exhaustive()
    }
}

impl ConvexClientTransport {
    /// Connects to a deployment (`CONVEX_URL`, e.g. `https://happy-otter-123.convex.cloud`).
    pub async fn connect(deployment_url: &str) -> Result<Self, ConvexApiError> {
        let client =
            convex::ConvexClient::new(deployment_url).await.map_err(|e| ConvexApiError::Transport(e.to_string()))?;
        Ok(Self { client })
    }

    /// Wraps an existing client.
    pub fn from_client(client: convex::ConvexClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ConvexTransport for ConvexClientTransport {
    async fn query(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        let mut c = self.client.clone();
        let r = c.query(function, args_map(args)?).await.map_err(|e| ConvexApiError::Transport(e.to_string()))?;
        result(function, r)
    }

    async fn mutation(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        let mut c = self.client.clone();
        let r = c.mutation(function, args_map(args)?).await.map_err(|e| ConvexApiError::Transport(e.to_string()))?;
        result(function, r)
    }

    async fn action(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        let mut c = self.client.clone();
        let r = c.action(function, args_map(args)?).await.map_err(|e| ConvexApiError::Transport(e.to_string()))?;
        result(function, r)
    }

    async fn subscribe(&self, function: &str, args: Json) -> Result<JsonStream, ConvexApiError> {
        let mut c = self.client.clone();
        let sub = c.subscribe(function, args_map(args)?).await.map_err(|e| ConvexApiError::Transport(e.to_string()))?;
        let name = function.to_owned();
        Ok(sub.map(move |r| result(&name, r)).boxed())
    }

    async fn set_auth(&self, token: Option<String>) {
        let mut c = self.client.clone();
        c.set_auth(token).await;
    }
}

/// One recorded call of [`MockTransport`].
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedCall {
    /// `query`, `mutation`, `action` or `subscribe`.
    pub kind: &'static str,
    /// Function name.
    pub function: String,
    /// Arguments as sent.
    pub args: Json,
}

/// A scripted transport for tests: responses are queued per function name and every call is
/// recorded. Arguments go through the same JSON → Convex conversion as production, so encoding
/// errors surface in tests too.
#[derive(Default)]
pub struct MockTransport {
    responses: Mutex<Vec<(String, VecDeque<Result<Json, ConvexApiError>>)>>,
    calls: Mutex<Vec<RecordedCall>>,
    auth: Mutex<Option<String>>,
}

impl std::fmt::Debug for MockTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockTransport").finish_non_exhaustive()
    }
}

impl MockTransport {
    /// An empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a response for `function`.
    pub fn respond(&self, function: &str, response: Result<Json, ConvexApiError>) -> &Self {
        let mut r = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        match r.iter_mut().find(|(f, _)| f == function) {
            Some((_, q)) => q.push_back(response),
            None => r.push((function.to_owned(), VecDeque::from([response]))),
        }
        self
    }

    /// Every call so far.
    pub fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The token last passed to `set_auth`.
    pub fn auth(&self) -> Option<String> {
        self.auth.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn call(&self, kind: &'static str, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        args_map(args.clone())?;
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(RecordedCall {
            kind,
            function: function.to_owned(),
            args,
        });
        let mut r = self.responses.lock().unwrap_or_else(|e| e.into_inner());
        r.iter_mut().find(|(f, _)| f == function).and_then(|(_, q)| q.pop_front()).unwrap_or_else(|| {
            Err(ConvexApiError::Function { function: function.to_owned(), message: "no mock response".into() })
        })
    }
}

#[async_trait]
impl ConvexTransport for MockTransport {
    async fn query(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        self.call("query", function, args)
    }

    async fn mutation(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        self.call("mutation", function, args)
    }

    async fn action(&self, function: &str, args: Json) -> Result<Json, ConvexApiError> {
        self.call("action", function, args)
    }

    async fn subscribe(&self, function: &str, args: Json) -> Result<JsonStream, ConvexApiError> {
        // Every queued response for the function becomes one stream item.
        args_map(args.clone())?;
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(RecordedCall {
            kind: "subscribe",
            function: function.to_owned(),
            args,
        });
        let items: Vec<_> = {
            let mut r = self.responses.lock().unwrap_or_else(|e| e.into_inner());
            r.iter_mut().find(|(f, _)| f == function).map(|(_, q)| q.drain(..).collect()).unwrap_or_default()
        };
        Ok(futures::stream::iter(items).boxed())
    }

    async fn set_auth(&self, token: Option<String>) {
        *self.auth.lock().unwrap_or_else(|e| e.into_inner()) = token;
    }
}
