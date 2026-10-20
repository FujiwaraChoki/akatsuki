//! The command palette's list (BRIEF "Quality bar": every action is reachable from the palette).
//!
//! Every shortcut marked `palette` in `packages/shortcuts/shortcuts.json` is a row, with its key
//! hint; the desktop adds settings and navigation rows that have no key (theme, key map, labels,
//! splits, sign-out). Running a row goes through the same dispatcher as its key.

use akatsuki_shortcuts::{Keymap, Scope, ids as sc, palette_entries};

use crate::config::{Density, KeymapPref};
use crate::keys;
use crate::theme::ThemePref;

/// Ids of desktop-only commands (no key binding).
pub mod ids {
    /// Sign out and wipe this device's caches.
    pub const SIGN_OUT: &str = "app.signOut";
    /// Quit.
    pub const QUIT: &str = "app.quit";
    /// Open the demo mailbox (when no server is configured).
    pub const DEMO: &str = "app.demo";
    /// Settings: AI providers.
    pub const SETTINGS_AI: &str = "settings.ai";
    /// Settings: labels.
    pub const SETTINGS_LABELS: &str = "settings.labels";
    /// Settings: snippets.
    pub const SETTINGS_SNIPPETS: &str = "settings.snippets";
    /// Rewrite in one style: `compose.rewrite:shorter` (the `compose.rewrite` picker's rows).
    pub const REWRITE_PREFIX: &str = "compose.rewrite:";
    /// Undo the last AI change to the draft.
    pub const AI_UNDO: &str = "compose.rewrite:undo";
    /// Retry one failed op: `ops.retry:<opId>`.
    pub const RETRY_PREFIX: &str = "ops.retry:";
    /// Dismiss one failed op: `ops.dismiss:<opId>`.
    pub const DISMISS_PREFIX: &str = "ops.dismiss:";
    /// "Keep this one" after a draft conflict: `drafts.keep:<version>`.
    pub const KEEP_DRAFT_PREFIX: &str = "drafts.keep:";
    /// Theme prefix (`theme.light`).
    pub const THEME: &str = "theme.";
    /// Key map prefix (`keymap.gmail`).
    pub const KEYMAP: &str = "keymap.";
    /// Density prefix (`density.compact`).
    pub const DENSITY: &str = "density.";
    /// Go to a label: `go.label:<labelKey>`.
    pub const GO_LABEL_PREFIX: &str = "go.label:";
    /// Go to a split tab: `go.split:<labelKey>`.
    pub const GO_SPLIT_PREFIX: &str = "go.split:";
    /// Switch account filter: `go.account:<accountId>` (empty = all accounts).
    pub const GO_ACCOUNT_PREFIX: &str = "go.account:";
}

/// One palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// The command to run.
    pub id: String,
    /// Label.
    pub label: String,
    /// Group heading.
    pub group: String,
    /// Key hint.
    pub hint: Option<String>,
    /// The current choice among options.
    pub checked: bool,
    /// Synonyms the filter also matches (from the key map).
    pub keywords: Vec<String>,
}

/// Palette shortcuts the desktop does not implement (none since send later, snippets and AI write
/// landed; kept so a new shared entry can be parked here until it is built).
pub const UNSUPPORTED: [&str; 0] = [];

/// What the palette needs to know about the app.
#[derive(Clone, Debug, Default)]
pub struct PaletteContext {
    /// Key map in use.
    pub keymap: KeymapPref,
    /// Theme in use.
    pub theme: ThemePref,
    /// Density in use.
    pub density: Density,
    /// `(labelKey, name)` of user labels.
    pub labels: Vec<(String, String)>,
    /// `(labelKey, name)` of split tabs.
    pub splits: Vec<(String, String)>,
    /// `(accountId, email)`.
    pub accounts: Vec<(String, String)>,
    /// Signed in (or in the demo).
    pub signed_in: bool,
    /// A conversation is open (thread commands apply).
    pub thread_open: bool,
    /// A composer is open (compose commands apply).
    pub composing: bool,
    /// Failed actions wait for Retry or Dismiss.
    pub failures: bool,
    /// Drafts are kept on this device.
    pub local_drafts: bool,
    /// The composer has AI changes to undo.
    pub ai_undo: bool,
}

/// Whether a palette shortcut of `scope` applies in this context, as its key would before the palette opened
/// (web and mobile filter the same way).
fn applies(scope: Scope, ctx: &PaletteContext) -> bool {
    match scope {
        Scope::Global | Scope::Mail | Scope::List | Scope::Palette => true,
        Scope::Thread => ctx.thread_open,
        Scope::Compose => ctx.composing,
        Scope::Search => false,
    }
}

/// Commands that only make sense in some states (D-409: the palette offers what this screen can do).
fn available(id: &str, ctx: &PaletteContext) -> bool {
    match id {
        sc::OPS_RETRY => ctx.failures,
        sc::COMPOSE_CONTINUE_DRAFT => ctx.local_drafts,
        _ => true,
    }
}

fn row(
    id: impl Into<String>,
    label: impl Into<String>,
    group: &str,
    hint: Option<String>,
    checked: bool,
) -> PaletteItem {
    PaletteItem { id: id.into(), label: label.into(), group: group.to_owned(), hint, checked, keywords: Vec::new() }
}

/// Every row, in display order.
pub fn palette_items(ctx: &PaletteContext) -> Vec<PaletteItem> {
    let keymap: Keymap = ctx.keymap.keymap();
    let mut items: Vec<PaletteItem> = Vec::new();
    if ctx.signed_in {
        items.extend(
            palette_entries()
                .filter(|s| applies(s.scope, ctx) && available(s.id, ctx) && !UNSUPPORTED.contains(&s.id))
                .map(|s| PaletteItem {
                    keywords: s.keywords.iter().map(|k| (*k).to_owned()).collect(),
                    ..row(s.id, s.label, s.group, keys::hint(s.id, keymap), false)
                }),
        );
        if ctx.composing {
            for style in crate::vm::ai::RewriteStyle::ALL {
                items.push(PaletteItem {
                    keywords: vec!["ai".into(), "rewrite".into()],
                    ..row(
                        format!("{}{}", ids::REWRITE_PREFIX, style.as_str()),
                        format!("Rewrite: {}", style.label()),
                        "Compose",
                        None,
                        false,
                    )
                });
            }
            if ctx.ai_undo {
                items.push(row(ids::AI_UNDO, "Undo the AI change", "Compose", None, false));
            }
        }
        for (key, name) in &ctx.splits {
            items.push(row(
                format!("{}{key}", ids::GO_SPLIT_PREFIX),
                format!("Go to split: {name}"),
                "Navigate",
                None,
                false,
            ));
        }
        for (key, name) in &ctx.labels {
            items.push(row(
                format!("{}{key}", ids::GO_LABEL_PREFIX),
                format!("Go to label: {name}"),
                "Navigate",
                None,
                false,
            ));
        }
        if ctx.accounts.len() > 1 {
            items.push(row(ids::GO_ACCOUNT_PREFIX, "Show all accounts", "Navigate", None, false));
            for (id, email) in &ctx.accounts {
                items.push(row(
                    format!("{}{id}", ids::GO_ACCOUNT_PREFIX),
                    format!("Show only {email}"),
                    "Navigate",
                    None,
                    false,
                ));
            }
        }
        items.push(row(ids::SETTINGS_AI, "AI providers", "Settings", None, false));
        items.push(PaletteItem {
            keywords: vec!["manage labels".into(), "folders".into()],
            ..row(ids::SETTINGS_LABELS, "Labels", "Settings", None, false)
        });
        items.push(PaletteItem {
            keywords: vec!["templates".into(), "canned".into()],
            ..row(ids::SETTINGS_SNIPPETS, "Snippets", "Settings", None, false)
        });
    }
    for t in [ThemePref::System, ThemePref::Light, ThemePref::Dark] {
        let id = format!("{}{}", ids::THEME, t.label().to_lowercase());
        items.push(row(id, format!("Theme: {}", t.label()), "Settings", None, ctx.theme == t));
    }
    for (k, label) in [(KeymapPref::Superhuman, "Superhuman"), (KeymapPref::Gmail, "Gmail")] {
        let id = format!("{}{}", ids::KEYMAP, label.to_lowercase());
        items.push(row(id, format!("Keyboard shortcuts: {label}"), "Settings", None, ctx.keymap == k));
    }
    for (d, label) in [(Density::Comfortable, "Comfortable"), (Density::Compact, "Compact")] {
        let id = format!("{}{}", ids::DENSITY, label.to_lowercase());
        items.push(row(id, format!("Density: {label}"), "Settings", None, ctx.density == d));
    }
    if ctx.signed_in {
        items.push(PaletteItem {
            keywords: vec!["log out".into(), "logout".into(), "sign off".into()],
            ..row(ids::SIGN_OUT, "Sign out", "Account", None, false)
        });
    } else {
        items.push(row(ids::DEMO, "Open the demo mailbox", "Account", None, false));
    }
    items.push(row(ids::QUIT, "Quit Akatsuki", "Account", None, false));
    items
}

/// Rows matching `query`: every word must appear in the group or label; labels that start with
/// the first word come first.
pub fn filter(items: Vec<PaletteItem>, query: &str) -> Vec<PaletteItem> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    if words.is_empty() {
        return items;
    }
    let mut found: Vec<(u8, PaletteItem)> = items
        .into_iter()
        .filter_map(|item| {
            let label = item.label.to_lowercase();
            let hay = format!("{} {label} {}", item.group.to_lowercase(), item.keywords.join(" ").to_lowercase());
            if !words.iter().all(|w| hay.contains(w)) {
                return None;
            }
            let rank = if label.starts_with(words[0]) {
                0
            } else if label.split_whitespace().any(|w| w.starts_with(words[0])) {
                1
            } else {
                2
            };
            Some((rank, item))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_shortcuts::SHORTCUTS;

    #[test]
    fn synonyms_from_the_key_map_match() {
        let items = palette_items(&PaletteContext { signed_in: true, ..ctx() });
        let first = |q: &str| filter(items.clone(), q).first().map(|i| i.id.clone());
        assert_eq!(first("new email").as_deref(), Some(akatsuki_shortcuts::ids::COMPOSE_NEW));
        assert_eq!(first("print").as_deref(), Some(akatsuki_shortcuts::ids::THREAD_OPEN_ORIGINAL));
    }

    fn ctx() -> PaletteContext {
        PaletteContext {
            signed_in: true,
            thread_open: true,
            composing: true,
            failures: true,
            local_drafts: true,
            labels: vec![("l:1".into(), "Receipts".into())],
            splits: vec![("split:a".into(), "VIP".into())],
            accounts: vec![("a1".into(), "me@example.com".into()), ("a2".into(), "me@work.example".into())],
            ..PaletteContext::default()
        }
    }

    #[test]
    fn every_palette_shortcut_is_listed_with_its_key() {
        let items = palette_items(&ctx());
        for s in SHORTCUTS.iter().filter(|s| s.palette && s.scope != Scope::Search && !UNSUPPORTED.contains(&s.id)) {
            let item = items.iter().find(|i| i.id == s.id).unwrap_or_else(|| panic!("{} missing", s.id));
            assert_eq!(item.label, s.label);
        }
        let archive = items.iter().find(|i| i.id == "thread.archive").unwrap();
        assert_eq!(archive.hint.as_deref(), Some("E"));
        assert!(items.iter().any(|i| i.id == "go.label:l:1"));
        assert!(items.iter().any(|i| i.id == "go.split:split:a"));
        assert!(items.iter().any(|i| i.id == "go.account:a2"));
        assert!(items.iter().any(|i| i.id == ids::SIGN_OUT));
        assert!(items.iter().find(|i| i.id == "theme.system").unwrap().checked);
    }

    #[test]
    fn rows_follow_the_context_and_skip_what_the_alpha_lacks() {
        let list = palette_items(&PaletteContext { thread_open: false, composing: false, ..ctx() });
        let scope_of = |id: &str| SHORTCUTS.iter().find(|s| s.id == id).map(|s| s.scope);
        assert!(list.iter().all(|i| scope_of(&i.id) != Some(Scope::Compose)), "no Send without a composer");
        assert!(list.iter().all(|i| scope_of(&i.id) != Some(Scope::Thread)), "no Reply without a thread");
        assert!(list.iter().any(|i| i.id == sc::COMPOSE_NEW));
        let composing = palette_items(&ctx());
        assert!(composing.iter().any(|i| i.id == sc::COMPOSE_SEND));
        for id in [
            sc::COMPOSE_SEND_LATER,
            sc::COMPOSE_SNIPPET,
            sc::COMPOSE_AI_WRITE,
            sc::COMPOSE_REWRITE,
            sc::COMPOSE_DICTATE,
        ] {
            assert!(composing.iter().any(|i| i.id == id), "{id} is listed");
        }
        assert!(composing.iter().any(|i| i.id == "compose.rewrite:shorter"));
        assert!(list.iter().all(|i| !i.id.starts_with(ids::REWRITE_PREFIX)), "rewrite rows need a composer");
        let calm = palette_items(&PaletteContext { failures: false, local_drafts: false, ..ctx() });
        assert!(calm.iter().all(|i| i.id != sc::OPS_RETRY), "Retry only with something to retry");
        assert!(calm.iter().all(|i| i.id != sc::COMPOSE_CONTINUE_DRAFT));
        assert!(composing.iter().any(|i| i.id == sc::OPS_RETRY));
        assert!(composing.iter().any(|i| i.id == ids::SETTINGS_SNIPPETS));
        assert!(composing.iter().any(|i| i.id == sc::LABELS_CREATE));
    }

    #[test]
    fn signed_out_palette_offers_settings_and_the_demo() {
        let items = palette_items(&PaletteContext::default());
        assert!(items.iter().all(|i| !i.id.starts_with("thread.")));
        assert!(items.iter().any(|i| i.id == ids::DEMO));
    }

    #[test]
    fn filtering_matches_every_word_and_ranks_prefixes() {
        let items = palette_items(&ctx());
        let found = filter(items.clone(), "arch");
        assert_eq!(found[0].id, "thread.archive");
        let found = filter(items.clone(), "theme dark");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "theme.dark");
        assert!(filter(items.clone(), "zzzz").is_empty());
        assert_eq!(filter(items.clone(), "").len(), items.len());
        assert_eq!(filter(items, "receipts")[0].id, "go.label:l:1");
    }
}
