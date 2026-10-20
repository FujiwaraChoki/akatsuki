//! The sidebar (lists, labels, accounts) and the header (title, split tabs, search).

use gpui::{Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_shortcuts::ids as sc;

use super::widgets::{count, field_frame, ghost, icon, kbd, label};
use super::workspace::Workspace;
use crate::backend::SessionState;
use crate::commands::ids as cmd;
use crate::theme::{fonts, styles};
use crate::vm::mail::Folder;

fn folder_icon(f: &Folder) -> &'static str {
    match f {
        Folder::Inbox => "inbox",
        Folder::Starred => "star",
        Folder::Snoozed => "clock",
        Folder::Reminders => "bell",
        Folder::Sent => "send",
        Folder::Drafts => "file",
        Folder::All => "all",
        Folder::Spam => "octagon",
        Folder::Trash => "trash",
        Folder::Label { .. } => "tag",
        Folder::Search { .. } => "search",
    }
}

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
        let demo = matches!(self.session, SessionState::SignedIn { demo: true, .. });
        let (labels, accounts) = {
            let store = self.store();
            (store.labels(None).unwrap_or_default(), store.accounts().unwrap_or_default())
        };
        let mut folders = div().flex().flex_col().gap(px(2.));
        for (i, f) in Folder::SYSTEM.iter().enumerate() {
            let active = self.vm.folder == *f;
            let n = f.label_key().and_then(|k| self.unread.get(k)).copied().unwrap_or(0);
            let show = matches!(f, Folder::Inbox | Folder::Drafts | Folder::Spam) && n > 0;
            let id = folder_command(f);
            folders = folders.child(
                ghost(&t, ("folder", i), active)
                    .child(icon(folder_icon(f), 15., if active { t.accent_text } else { t.muted }))
                    .child(div().flex_1().child(f.title()))
                    .when(show, |d| {
                        d.child(div().text_size(px(styles::CAPTION.size)).text_color(t.muted).child(count(n)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| this.run(id, window, cx))),
            );
        }
        let user_labels: Vec<_> = labels.iter().filter(|l| !l.hidden && l.role.is_none()).collect();
        let mut label_list = div().flex().flex_col().gap(px(2.));
        for (i, l) in user_labels.iter().enumerate() {
            let key = l.label_key.clone();
            let active = matches!(&self.vm.folder, Folder::Label { key: k, .. } if *k == key);
            let n = self.unread.get(&key).copied().unwrap_or(0);
            let multi = accounts.len() > 1;
            let acct = accounts
                .iter()
                .find(|a| a.id == l.account_id)
                .map(|a| a.display_name.clone().unwrap_or_else(|| a.email.clone()));
            label_list = label_list.child(
                ghost(&t, ("label", i), active)
                    .child(icon("tag", 14., if active { t.accent_text } else { t.muted }))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(l.name.clone()))
                    .when(multi, |d| {
                        d.child(
                            div()
                                .text_size(px(styles::CAPTION.size))
                                .text_color(t.faint)
                                .child(acct.unwrap_or_default()),
                        )
                    })
                    .when(n > 0, |d| {
                        d.child(div().text_size(px(styles::CAPTION.size)).text_color(t.muted).child(count(n)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let id = format!("{}{key}", cmd::GO_LABEL_PREFIX);
                        this.run(&id, window, cx);
                    })),
            );
        }
        let mut account_list = div().flex().flex_col().gap(px(2.));
        if accounts.len() > 1 {
            let all_active = self.vm.account.is_none();
            account_list = account_list.child(
                ghost(&t, "acct-all", all_active)
                    .child(icon("all", 14., t.muted))
                    .child("All accounts")
                    .on_click(cx.listener(|this, _, window, cx| this.run(cmd::GO_ACCOUNT_PREFIX, window, cx))),
            );
            for (i, a) in accounts.iter().enumerate() {
                let id = a.id.as_str().to_owned();
                let active = self.vm.account.as_deref() == Some(id.as_str());
                account_list = account_list.child(
                    ghost(&t, ("acct", i), active)
                        .child(icon("user", 14., t.muted))
                        .child(div().flex_1().min_w(px(0.)).truncate().child(a.email.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let c = format!("{}{id}", cmd::GO_ACCOUNT_PREFIX);
                            this.run(&c, window, cx);
                        })),
                );
            }
        }
        let compose_hint = crate::keys::hint(sc::COMPOSE_NEW, self.settings.keymap.keymap()).unwrap_or_default();
        div()
            .id("sidebar")
            .w(px(232.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .px(px(12.))
            .py(px(16.))
            .gap(px(18.))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .px(px(6.))
                    .child(
                        div()
                            .font_family(fonts::SERIF)
                            .text_size(px(styles::SUBJECT.size))
                            .line_height(px(styles::SUBJECT.size * styles::SUBJECT.line_height))
                            .child("Akatsuki"),
                    )
                    .child(div().w(px(6.)).h(px(6.)).mb(px(7.)).rounded_full().bg(t.accent))
                    .when(demo, |d| d.child(div().mb(px(4.)).child(label(&t, "Demo")))),
            )
            .child(
                super::widgets::button(&t, "compose", "Compose", true)
                    .w_full()
                    .child(div().ml(px(8.)).opacity(0.75).text_size(px(styles::CAPTION.size)).child(compose_hint))
                    .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_NEW, window, cx))),
            )
            .child(div().flex().flex_col().gap(px(6.)).child(div().px(px(10.)).child(label(&t, "Mail"))).child(folders))
            .when(!user_labels.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().px(px(10.)).child(label(&t, "Labels")))
                        .child(label_list),
                )
            })
            .when(accounts.len() > 1, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().px(px(10.)).child(label(&t, "Accounts")))
                        .child(account_list),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(styles::META.size))
                            .text_color(t.muted)
                            .child(self.email.clone()),
                    )
                    .child(
                        div()
                            .id("open-settings")
                            .cursor_pointer()
                            .p(px(4.))
                            .rounded(px(akatsuki_tokens::radius::SM))
                            .hover(|s| s.bg(t.accent_wash))
                            .child(icon("settings", 16., t.muted))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::SETTINGS_OPEN, window, cx))),
                    ),
            )
    }

    pub(crate) fn render_header(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = self.theme;
        let focused = self.search.focus_handle(cx).is_focused(window);
        let search_hint = crate::keys::hint(sc::SEARCH_OPEN, self.settings.keymap.keymap()).unwrap_or_default();
        let tabs =
            if self.vm.folder == Folder::Inbox && self.vm.open.is_none() { self.vm.tabs.clone() } else { Vec::new() };
        let mut tab_row = div().flex().items_end().gap(px(20.)).h(px(34.));
        for (i, tab) in tabs.iter().enumerate() {
            let active = self.vm.tab.as_ref() == Some(&tab.key);
            let n = self.unread.get(&tab.key).copied().unwrap_or(0);
            let key = tab.key.clone();
            tab_row = tab_row.child(
                div()
                    .id(("tab", i))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pb(px(8.))
                    .cursor_pointer()
                    .border_b_2()
                    .border_color(if active { t.accent_text } else { gpui::transparent_black() })
                    .text_color(if active { t.fg } else { t.muted })
                    .font_weight(if active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                    .child(tab.name.clone())
                    .when(tab.show_count && n > 0, |d| {
                        d.child(div().text_size(px(styles::CAPTION.size)).text_color(t.link).child(count(n)))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let id = format!("{}{key}", cmd::GO_SPLIT_PREFIX);
                        this.run(&id, window, cx);
                    })),
            );
        }
        let title = self.vm.folder.title();
        div()
            .flex()
            .flex_col()
            .px(px(akatsuki_tokens::space::S3))
            .pt(px(14.))
            .border_b_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .h(px(40.))
                    .child(
                        div()
                            .font_family(fonts::SERIF)
                            .text_size(px(styles::HEADING.size))
                            .line_height(px(32.))
                            .child(title),
                    )
                    .child(div().flex_1())
                    .child(
                        field_frame(&t, focused)
                            .w(px(420.))
                            .child(icon("search", 14., t.muted))
                            .child(div().flex_1().child(self.search.clone()))
                            .when(!focused, |d| d.child(kbd(&t, search_hint))),
                    ),
            )
            .when_some(self.search_error.clone(), |d, e| {
                d.child(div().pt(px(4.)).text_size(px(styles::META.size)).text_color(t.link).child(e))
            })
            .child(if tabs.is_empty() { div().h(px(10.)) } else { tab_row })
    }
}
