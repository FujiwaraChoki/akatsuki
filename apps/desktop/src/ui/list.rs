//! The thread list: virtualized (`uniform_list` renders only the visible rows, read from the
//! mirror per frame), keyboard cursor, selection and an empty state with the crimson orbs.

use std::ops::Range;

use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px, uniform_list};

use akatsuki_core::time::now_ms;

use super::widgets::{icon, label};
use super::workspace::Workspace;
use crate::theme::{fonts, styles};
use crate::vm::mail::Folder;
use crate::vm::time;

impl Workspace {
    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let t = self.theme;
        let ids: Vec<String> = self.vm.ids.get(range.clone()).map(<[String]>::to_vec).unwrap_or_default();
        let rows = self.store().summaries(&ids).unwrap_or_default();
        let accounts = self.store().accounts().unwrap_or_default();
        let multi_account = self.vm.account.is_none() && accounts.len() > 1;
        let account_tag = |id: &str| {
            accounts
                .iter()
                .find(|a| a.id.as_str() == id)
                .map(|a| a.display_name.clone().unwrap_or_else(|| a.email.split('@').nth(1).unwrap_or("").to_owned()))
                .unwrap_or_default()
        };
        let now = now_ms();
        let row_h = self.settings.density.row_height();
        rows.into_iter()
            .enumerate()
            .map(|(i, r)| {
                let ix = range.start + i;
                let is_cursor = ix == self.vm.cursor;
                let is_selected = self.vm.selected.contains(&r.id);
                let from = r
                    .last_from
                    .as_ref()
                    .map(|a| a.name.clone().unwrap_or_else(|| a.email.clone()))
                    .unwrap_or_else(|| "(no sender)".into());
                let from = if r.message_count > 1 { format!("{from}  {}", r.message_count) } else { from };
                let when = if self.vm.folder == Folder::Snoozed {
                    r.snoozed_until.map(|s| time::list_time(s, now, &self.tz)).unwrap_or_default()
                } else {
                    time::list_time(r.sort_at, now, &self.tz)
                };
                let subject = if r.subject.trim().is_empty() { "(no subject)".to_owned() } else { r.subject.clone() };
                let weight = if r.unread { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
                div()
                    .id(("row", ix))
                    .w_full()
                    .h(px(row_h))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pl(px(akatsuki_tokens::space::S3 - 2.))
                    .pr(px(akatsuki_tokens::space::S3))
                    .border_l_2()
                    .border_color(if is_cursor { t.accent_text } else { gpui::transparent_black() })
                    .when(is_selected, |d| d.bg(t.accent_tint))
                    .when(is_cursor && !is_selected, |d| d.bg(t.accent_wash))
                    .cursor_pointer()
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().flex_none().when(r.unread, |d| d.bg(t.accent)))
                    .child(
                        div()
                            .w(px(190.))
                            .flex_none()
                            .truncate()
                            .font_weight(weight)
                            .text_color(if r.unread { t.fg } else { t.secondary })
                            .child(from),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .gap(px(8.))
                            .overflow_hidden()
                            .child(div().flex_none().max_w(px(420.)).truncate().font_weight(weight).child(subject))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_color(t.muted)
                                    .child(super::reading::one_line(&r.snippet)),
                            ),
                    )
                    .when(r.needs_reply, |d| d.child(label(&t, "Reply")))
                    .when(multi_account, |d| {
                        d.child(
                            div()
                                .text_size(px(styles::CAPTION.size))
                                .text_color(t.faint)
                                .child(account_tag(&r.account_id)),
                        )
                    })
                    .when(r.has_attachments, |d| d.child(icon("paperclip", 13., t.muted)))
                    .when(r.starred, |d| d.child(icon("star", 13., t.accent_text)))
                    .when(r.pending, |d| d.child(div().w(px(4.)).h(px(4.)).rounded_full().bg(t.faint)))
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .text_size(px(styles::META.size))
                            .text_color(if r.unread { t.fg } else { t.muted })
                            .child(when),
                    )
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        this.vm.cursor = ix;
                        if e.modifiers().secondary() {
                            this.run(akatsuki_shortcuts::ids::SELECT_TOGGLE, window, cx);
                        } else {
                            this.run(akatsuki_shortcuts::ids::NAV_OPEN, window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }

    pub(crate) fn render_list(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        if self.vm.ids.is_empty() {
            let (title, line) = match &self.vm.folder {
                Folder::Inbox => ("Inbox zero", "Nothing left. Enjoy the dawn."),
                Folder::Search { .. } => ("No results", "Try fewer words, or from:, subject:, has:attachment."),
                _ => ("Nothing here", "This list is empty."),
            };
            return div()
                .size_full()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .overflow_hidden()
                .child(super::overlays::orbs(&t, self.reduce_motion, 3))
                .child(div().font_family(fonts::SERIF).text_size(px(styles::EMPTY.size)).child(title))
                .child(div().text_color(t.muted).child(line))
                .into_any_element();
        }
        let selection = self.vm.selected.len();
        let n = self.vm.ids.len();
        let km = self.settings.keymap.keymap();
        let hints: Vec<String> = [
            (akatsuki_shortcuts::ids::THREAD_ARCHIVE, "archive"),
            (akatsuki_shortcuts::ids::THREAD_TRASH, "trash"),
            (akatsuki_shortcuts::ids::THREAD_TOGGLE_READ, "read"),
            (akatsuki_shortcuts::ids::THREAD_LABEL, "label"),
            (akatsuki_shortcuts::ids::NAV_BACK, "clear"),
        ]
        .iter()
        .filter_map(|(id, what)| crate::keys::hint(id, km).map(|k| format!("{k} {what}")))
        .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(selection > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .h(px(32.))
                        .px(px(akatsuki_tokens::space::S3))
                        .bg(t.accent_wash)
                        .text_size(px(styles::META.size))
                        .child(format!("{selection} selected"))
                        .child(div().text_color(t.muted).child(hints.join(" · "))),
                )
            })
            .child(
                uniform_list("threads", n, cx.processor(|this, range, window, cx| this.render_rows(range, window, cx)))
                    .track_scroll(self.list_scroll.clone())
                    .flex_1()
                    .w_full(),
            )
            .into_any_element()
    }
}
