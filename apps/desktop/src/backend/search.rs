//! Search beyond the local index (§11.2): Convex `search.threads` (headers and snippets of
//! everything synced, merged with the local hits by thread id) and the provider search
//! (`GET /v1/search`, "Search the full mailbox"), whose hits the worker upserts into Convex so they
//! reach the mirror through the feed. Provider data is never copied anywhere else.

use std::sync::Arc;
use std::time::Duration;

use akatsuki_core::convex::client::{PaginationOpts, SearchThreadsArgs};
use akatsuki_core::time::now_ms;

use super::{Backend, Event};

/// Where a set of hits came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSource {
    /// Convex `search.threads`.
    Server,
    /// The mail providers (`GET /v1/search`).
    Provider,
}

/// Pages of provider results per account, at most.
const PROVIDER_PAGES: usize = 3;

impl Backend {
    /// Runs `q` on Convex (§11.2 backend 1). `nowAt` is rounded down to the minute so the query
    /// cache stays effective.
    pub fn search_server(self: &Arc<Self>, q: &str, account: Option<String>) {
        let Some(r) = self.remote() else { return };
        let query = q.to_owned();
        let me = self.clone();
        self.rt.spawn(async move {
            let args = SearchThreadsArgs {
                q: query.chars().take(500).collect(),
                now_at: now_ms() - now_ms().rem_euclid(60_000),
                account_id: account.as_deref().map(Into::into),
                pagination_opts: PaginationOpts { num_items: 50, cursor: None },
            };
            let event = match r.api.search_threads(&args).await {
                Ok(res) => Event::SearchResults {
                    query,
                    source: SearchSource::Server,
                    hits: res.page.into_iter().map(|t| t.id.into_string()).collect(),
                    unsupported: res.unsupported,
                    error: None,
                },
                // Offline or not entitled: the local results stand on their own.
                Err(_) => Event::SearchResults {
                    query,
                    source: SearchSource::Server,
                    hits: vec![],
                    unsupported: vec![],
                    error: None,
                },
            };
            me.emit(event);
        });
    }

    /// "Search the full mailbox" (§11.2 backend 2): every account whose provider searches (or one
    /// account), up to three pages each. Hits arrive in the mirror through the feed; the list shows
    /// them as they land.
    pub fn search_provider(self: &Arc<Self>, q: &str, account: Option<String>) {
        let Some(r) = self.remote() else { return };
        let accounts: Vec<String> = self
            .store
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.caps.server_search && account.as_ref().is_none_or(|x| x == a.id.as_str()))
            .map(|a| a.id.into_string())
            .collect();
        let query = q.to_owned();
        let me = self.clone();
        self.rt.spawn(async move {
            let mut hits: Vec<String> = Vec::new();
            let mut error = None;
            for account in accounts {
                let mut cursor: Option<String> = None;
                for _ in 0..PROVIDER_PAGES {
                    match r.mail.search(&account, &query, cursor.as_deref()).await {
                        Ok(res) => {
                            hits.extend(res.thread_ids.into_iter().map(|t| t.into_string()));
                            cursor = res.next_cursor;
                            if cursor.is_none() {
                                break;
                            }
                        }
                        Err(e) => {
                            error = Some(e.to_string());
                            break;
                        }
                    }
                }
            }
            hits.dedup();
            // The worker upserted the hits into Convex before answering; give the feed a moment.
            tokio::time::sleep(Duration::from_millis(300)).await;
            me.emit(Event::SearchResults { query, source: SearchSource::Provider, hits, unsupported: vec![], error });
        });
    }
}
