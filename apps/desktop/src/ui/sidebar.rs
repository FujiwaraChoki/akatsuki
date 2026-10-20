//! The sidebar (§3.1.1: account switcher, mailboxes with their keys and counts, labels,
//! accounts, footer hints) and the 44 px top bar (§3.1.2: split tabs or the mailbox name, the
//! bulk-selection bar, search and compose hints; `/` turns it into the search field).

use gpui::{AnyElement, Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_shortcuts::ids as sc;

use super::widgets::{count_el, crumb, ghost, icon, key_text, keys, label};
use super::workspace::Workspace;
use crate::backend::SessionState;
use crate::commands::ids as cmd;
use crate::keys::caps;
use crate::theme::{fonts, styles};
use crate::vm::mail::Folder;

fn folder_command(f: &Folder) -> &'static str {
    match f {
        Folder::Inbox => sc::GO_INBOX,
        Folder::Starred => sc::GO_STARRED,
        Folder::Snoozed => sc::GO_SNOOZED,
        Folder::Reminders => sc::GO_REMINDERS,
        Folder::Sent => sc::GO_SENT,
        Folder::Drafts => sc::GO_DRAFTS,
        Folder::All => sc::GO_ALL,
        Folder::Spam => sc::GO_SPAM,
        Folder::Trash => sc::GO_TRASH,
        Folder::Label { .. } | Folder::Search { .. } => sc::GO_LABEL,
    }
}

impl Workspace {
    pub(crate) fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let demo = matches!(self.session, SessionState::SignedIn { demo: true, .. });
        let (labels, accounts) = {
            let store = self.store();
            (store.labels(None).unwrap_or_default(), store.accounts().unwrap_or_default())
        };
        let mut folders = div().flex().flex_col().gap(px(1.)).mt(px(4.));
        for (i, f) in Folder::SYSTEM.iter().enumerate() {
            let active = self.vm.folder == *f;
            let n = f.label_key().and_then(|k| self.unread.get(k)).copied().unwrap_or(0);
            let show = matches!(f, Folder::Inbox | Folder::Drafts | Folder::Spam);
            let id = folder_command(f);
            folders = folders.child(
                ghost(&t, ("folder", i), active)
                    .child(div().flex_1().child(f.title()))
                    .child(key_text(&t, &caps(id, km)))
                    .child(count_el(&t, if show { n } else { 0 }))
                    .on_click(cx.listener(move |this, _, window, cx| this.run(id, window, cx))),
            );
        }
        let user_labels: Vec<_> = labels.iter().filter(|l| !l.hidden && l.role.is_none()).collect();
        let mut label_list = div().flex().flex_col().gap(px(1.));
        for (i, l) in user_labels.iter().enumerate() {
            let key = l.label_key.clone();
            let active = matches!(&self.vm.folder, Folder::Label { key: k, .. } if *k == key);
            let n = self.unread.get(&key).copied().unwrap_or(0);
            label_list = label_list.child(
                ghost(&t, ("label", i), active)
                    .h(px(26.))
                    .child(div().ml(px(4.)).size(px(8.)).rounded(px(2.)).border_1().border_color(t.muted))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(l.name.clone()))
                    .child(count_el(&t, n))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let id = format!("{}{key}", cmd::GO_LABEL_PREFIX);
                        this.run(&id, window, cx);
                    })),
            );
        }
        let mut account_list = div().flex().flex_col().gap(px(1.));
        for (i, a) in accounts.iter().enumerate() {
            let id = a.id.as_str().to_owned();
            let active = self.vm.account.as_deref() == Some(id.as_str());
            // The tile names the mailbox: its display name, else its domain.
            let letter = a
                .display_name
                .as_deref()
                .filter(|n| !n.trim().is_empty())
                .or_else(|| a.email.split('@').nth(1))
                .and_then(|n| n.chars().next())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            let kind = a.display_name.clone().unwrap_or_default();
            account_list = account_list.child(
                ghost(&t, ("acct", i), active)
                    .h(px(26.))
                    .text_size(px(styles::META.size))
                    .child(
                        div()
                            .flex_none()
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(akatsuki_tokens::radius::SM))
                            .border_1()
                            .border_color(t.line_strong)
                            .bg(t.bg)
                            .font_family(fonts::MONO)
                            .text_size(px(styles::CAPTION.size))
                            .text_color(t.secondary)
                            .child(letter),
                    )
                    .child(div().flex_1().min_w(px(0.)).truncate().child(a.email.clone()))
                    .child(div().text_size(px(styles::CAPTION.size)).text_color(t.muted).child(kind))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Clicking the account in use shows every account again.
                        let c = if this.vm.account.as_deref() == Some(id.as_str()) {
                            cmd::GO_ACCOUNT_PREFIX.to_owned()
                        } else {
                            format!("{}{id}", cmd::GO_ACCOUNT_PREFIX)
                        };
                        this.run(&c, window, cx);
                    })),
            );
        }
        let switcher = match self.vm.account.as_ref().and_then(|a| accounts.iter().find(|x| x.id.as_str() == a)) {
            Some(a) => a.email.clone(),
            None if accounts.len() > 1 => "All inboxes".to_owned(),
            None => accounts.first().map(|a| a.email.clone()).unwrap_or_else(|| "Inbox".into()),
        };
        let hint_row = |text: &str, id: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(styles::META.size))
                .text_color(t.tertiary)
                .child(keys(&t, &caps(id, km)))
                .child(text.to_owned())
        };
        div()
            .id("sidebar")
            .w(px(akatsuki_tokens::layout::SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .px(px(8.))
            .pb(px(12.))
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            .overflow_y_scroll()
            .child(
                div()
                    .id("switcher")
                    .flex_none()
                    .h(px(akatsuki_tokens::layout::TOOLBAR_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    // Clear of the macOS traffic lights (the title bar is transparent there).
                    .when(cfg!(target_os = "macos"), |d| d.pl(px(76.)))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .child(div().min_w(px(0.)).truncate().child(switcher))
                    .child(icon("chevron-down", 12., t.muted))
                    .when(demo, |d| d.child(div().flex_1()).child(crumb(&t, "Demo")))
                    .on_click(cx.listener(|this, _, window, cx| {
                        // The palette lists every account switch.
                        this.run(sc::PALETTE_OPEN, window, cx);
                    })),
            )
            .child(folders)
            .when(!self.vm.tabs.is_empty(), |d| {
                let mut splits = div().flex().flex_col().gap(px(1.));
                for (i, tab) in self.vm.tabs.iter().enumerate() {
                    let key = tab.key.clone();
                    let active = self.vm.folder == Folder::Inbox && self.vm.tab.as_ref() == Some(&key);
                    let n = self.unread.get(&key).copied().unwrap_or(0);
                    splits = splits.child(
                        ghost(&t, ("split", i), active)
                            .h(px(26.))
                            .child(div().flex_1().min_w(px(0.)).truncate().child(tab.name.clone()))
                            .child(count_el(&t, if tab.show_count { n } else { 0 }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run(&format!("{}{key}", cmd::GO_SPLIT_PREFIX), window, cx)
                            })),
                    );
                }
                d.child(div().mt(px(20.)).mb(px(4.)).px(px(12.)).child(label(&t, "Splits"))).child(splits)
            })
            .when(!user_labels.is_empty(), |d| {
                d.child(div().mt(px(20.)).mb(px(4.)).px(px(12.)).child(label(&t, "Labels"))).child(label_list)
            })
            .when(accounts.len() > 1, |d| {
                d.child(div().mt(px(20.)).mb(px(4.)).px(px(12.)).child(label(&t, "Accounts"))).child(account_list)
            })
            .child(div().flex_1().min_h(px(20.)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(12.))
                    .child(hint_row("Commands", sc::PALETTE_OPEN))
                    .child(hint_row("Shortcuts", sc::HELP_SHORTCUTS))
                    .child(
                        div()
                            .id("open-settings")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_size(px(styles::META.size))
                            .text_color(t.tertiary)
                            .child(keys(&t, &caps(sc::SETTINGS_OPEN, km)))
                            .child("Settings")
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::SETTINGS_OPEN, window, cx))),
                    ),
            )
    }

    /// The 44 px top bar over the list and reader.
    pub(crate) fn render_header(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let searching = self.search.focus_handle(cx).is_focused(window)
            || matches!(self.vm.folder, Folder::Search { .. })
            || self.search_error.is_some();
        let bar = div()
            .relative()
            .flex_none()
            .h(px(akatsuki_tokens::layout::TOOLBAR_HEIGHT))
            .flex()
            .items_center()
            .gap(px(16.))
            .pl(px(20.))
            .pr(px(12.))
            .border_b_1()
            .border_color(t.line)
            .bg(t.bg);
        if searching {
            let error = self.search_error.clone();
            return bar
                .child(icon("search", 14., t.muted))
                .child(div().flex_1().min_w(px(0.)).text_size(px(14.)).child(self.search.clone()))
                .when_some(error, |d, e| {
                    d.child(div().flex_none().text_size(px(styles::META.size)).text_color(t.accent_text).child(e))
                })
                .children(self.render_search_status(cx))
                .child(keys(&t, &caps(sc::SEARCH_CLOSE, km)))
                .into_any_element();
        }
        // Bulk selection replaces the tabs in place.
        let selected = self.vm.selected.len();
        let left: AnyElement = if selected > 0 {
            let mut row =
                div().flex().items_center().gap(px(10.)).text_size(px(styles::META.size)).text_color(t.tertiary).child(
                    div().text_color(t.fg).font_weight(FontWeight::MEDIUM).child(format!("{selected} selected")),
                );
            for (id, what) in [
                (sc::THREAD_ARCHIVE, "Archive"),
                (sc::THREAD_SNOOZE, "Snooze"),
                (sc::THREAD_LABEL, "Label"),
                (sc::THREAD_TRASH, "Trash"),
            ] {
                row = row.child(div().flex().items_center().gap(px(5.)).child(keys(&t, &caps(id, km))).child(what));
            }
            row.child(
                div().flex().items_center().gap(px(5.)).child(keys(&t, &caps(sc::NAV_BACK, km))).child("to clear"),
            )
            .into_any_element()
        } else {
            let tabs = if self.vm.folder == Folder::Inbox { self.vm.tabs.clone() } else { Vec::new() };
            let mut row = div().h_full().flex().items_center().gap(px(20.));
            let tab = |active: bool, name: String, n: Option<usize>| {
                div()
                    .relative()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .text_color(if active { t.fg } else { t.muted })
                    .child(name)
                    .when_some(n.filter(|n| *n > 0), |d, n| {
                        d.child(
                            div()
                                .font_family(fonts::MONO)
                                .text_size(px(styles::CAPTION.size))
                                .text_color(t.muted)
                                .child(super::widgets::count(n)),
                        )
                    })
                    .when(active, |d| d.child(div().absolute().left_0().bottom_0().w_full().h(px(2.)).bg(t.fg)))
            };
            if tabs.is_empty() {
                row = row.child(tab(true, self.vm.folder.title(), None));
            } else {
                for (i, s) in tabs.iter().enumerate() {
                    let active = self.vm.tab.as_ref() == Some(&s.key);
                    let n = s.show_count.then(|| self.unread.get(&s.key).copied().unwrap_or(0));
                    let key = s.key.clone();
                    row = row.child(tab(active, s.name.clone(), n).id(("tab", i)).on_click(cx.listener(
                        move |this, _, window, cx| this.run(&format!("{}{key}", cmd::GO_SPLIT_PREFIX), window, cx),
                    )));
                }
                row = row.child(keys(&t, &caps(sc::SPLIT_NEXT, km)));
            }
            row.into_any_element()
        };
        let hint = |id: &'static str, glyph: &'static str, text: &'static str, command: &'static str| {
            let surface = t.surface;
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(28.))
                .px(px(8.))
                .rounded(px(akatsuki_tokens::radius::MD))
                .cursor_pointer()
                .text_size(px(styles::META.size))
                .text_color(t.muted)
                .hover(move |s| s.bg(surface))
                .child(icon(glyph, 14., t.muted))
                .child(text)
                .child(keys(&t, &caps(command, km)))
                .on_click(cx.listener(move |this, _, window, cx| this.run(command, window, cx)))
        };
        bar.child(div().h_full().flex_1().min_w(px(0.)).flex().items_center().child(left))
            .child(hint("hint-search", "search", "Search", sc::SEARCH_OPEN))
            .child(hint("hint-compose", "compose", "Compose", sc::COMPOSE_NEW))
            .into_any_element()
    }

    /// Next to the search field: where results come from, and "Search the full mailbox"
    /// (§11.2 backend 2) when the server could not apply part of the query or the user wants more.
    fn render_search_status(&mut self, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let Folder::Search { query } = &self.vm.folder else { return None };
        let st = &self.search_state;
        if st.query != *query {
            return None;
        }
        let text = if st.provider_pending {
            "Searching the full mailbox…".to_owned()
        } else if let Some(e) = &st.provider_error {
            e.clone()
        } else if st.provider_done {
            "Includes the full mailbox".to_owned()
        } else if st.server_pending {
            "Checking the server…".to_owned()
        } else if !st.unsupported.is_empty() {
            format!("{} needs the full mailbox", st.unsupported.join(", "))
        } else {
            String::new()
        };
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(styles::META.size))
                .text_color(t.muted)
                .child(text)
                .when(!st.provider_pending && !st.provider_done, |d| {
                    d.child(
                        div()
                            .id("search-full")
                            .cursor_pointer()
                            .text_color(t.fg)
                            .underline()
                            .child("Search the full mailbox")
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::SEARCH_FULL_MAILBOX, window, cx))),
                    )
                })
                .into_any_element(),
        )
    }
}
