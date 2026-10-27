//! The thread list (§3.1.4): virtualized (`uniform_list` renders only the visible rows, read from
//! the mirror per frame), fixed column tracks `16 | 168 | 1fr | auto | 72`, the dawn cursor bar,
//! ink unread dots, key hints on the cursor row and hover actions; quiet empty states and the
//! inbox-zero horizon.

use std::ops::Range;

use gpui::{AnimationExt, AnyElement, Context, FontWeight, Window, div, prelude::*, px, uniform_list};

use akatsuki_core::time::now_ms;
use akatsuki_shortcuts::ids as sc;

use super::widgets::{chip, icon, keys, triage_label};
use super::workspace::Workspace;
use crate::keys::caps;
use crate::theme::{fonts, styles};
use crate::vm::mail::Folder;
use crate::vm::time;

/// The sender column.
const SENDER_W: f32 = 168.;
/// The time column.
const TIME_W: f32 = 72.;

impl Workspace {
    fn render_rows(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
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
        let compact = row_h < 40.;
        // The cursor bar is dawn while the list owns the keys, and drops to grey otherwise (§2.8).
        let list_has_keys = self.overlay == super::workspace::Overlay::None && !self.compose_has_focus(window, cx);
        let cursor_hints: Vec<String> =
            [sc::THREAD_ARCHIVE, sc::THREAD_SNOOZE, sc::THREAD_REPLY].iter().flat_map(|id| caps(id, km)).collect();
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
                let is_draft = r.label_keys.iter().any(|k| k == "sys:drafts");
                let when = if self.vm.folder == Folder::Snoozed {
                    r.snoozed_until.map(|s| time::list_time(s, now, &self.tz)).unwrap_or_default()
                } else {
                    time::list_time(r.sort_at, now, &self.tz)
                };
                let subject = if r.subject.trim().is_empty() { "(no subject)".to_owned() } else { r.subject.clone() };
                let snippet = super::reading::one_line(&r.snippet);
                let mut chips: Vec<String> = Vec::new();
                if let Some(c) = r.triage.as_ref().filter(|c| !matches!(c.as_str(), "other" | "personal")) {
                    chips.push(triage_label(c));
                }
                if r.needs_reply {
                    chips.push("Needs reply".into());
                }
                let group = gpui::SharedString::from(format!("row-{ix}"));
                let (surface, surface_strong) = (t.surface, t.surface_strong);
                let id = r.id.clone();
                div()
                    .id(("row", ix))
                    .group(group.clone())
                    .relative()
                    .w_full()
                    .h(px(row_h))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pl(px(12.))
                    .pr(px(16.))
                    .border_b_1()
                    .border_color(t.line)
                    .when(r.pending, |d| d.opacity(0.6))
                    .when(is_cursor, |d| d.bg(surface_strong))
                    .when(is_selected && !is_cursor, |d| d.bg(surface))
                    .when(!is_cursor, |d| d.hover(move |s| s.bg(surface)))
                    .cursor_pointer()
                    .when(is_cursor, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(2.))
                                .bg(if list_has_keys { t.dawn } else { t.stroke }),
                        )
                    })
                    // Dot gutter: the ink unread dot, or a check when picked.
                    .child(div().w(px(16.)).flex_none().flex().items_center().justify_center().map(|d| {
                        if is_selected {
                            d.child(icon("check", 12., t.fg))
                        } else if r.unread {
                            d.child(div().size(px(6.)).rounded_full().bg(t.fg))
                        } else {
                            d
                        }
                    }))
                    .child(
                        div()
                            .w(px(SENDER_W))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .overflow_hidden()
                            .when(is_draft, |d| d.child(div().flex_none().text_color(t.tertiary).child("Draft ·")))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .when(r.unread, |d| d.font_weight(FontWeight::MEDIUM).text_color(t.fg))
                                    .when(!r.unread, |d| d.text_color(t.secondary))
                                    .child(from),
                            )
                            .when(r.message_count > 1, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .font_family(fonts::MONO)
                                        .text_size(px(styles::CAPTION.size))
                                        .text_color(t.muted)
                                        .child(r.message_count.to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(
                                div()
                                    .flex()
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .flex_none()
                                            .max_w(px(480.))
                                            .truncate()
                                            .text_color(t.fg)
                                            .when(r.unread, |d| d.font_weight(FontWeight::MEDIUM))
                                            .child(subject),
                                    )
                                    .when(!snippet.is_empty() && !compact, |d| {
                                        d.child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .truncate()
                                                .text_color(if is_cursor { t.tertiary } else { t.muted })
                                                .child(format!(" — {snippet}")),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .when(!compact, |d| d.children(chips.iter().take(2).map(|c| chip(&t, c))))
                            .when(multi_account, |d| {
                                d.child(
                                    div()
                                        .text_size(px(styles::CAPTION.size))
                                        .text_color(t.muted)
                                        .child(account_tag(&r.account_id)),
                                )
                            })
                            .when(r.has_attachments, |d| d.child(icon("paperclip", 14., t.muted)))
                            .when(r.starred, |d| d.child(icon("star", 14., t.fg))),
                    )
                    .child(
                        div()
                            .relative()
                            .w(px(TIME_W))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .map(|d| {
                                if is_cursor {
                                    d.child(keys(&t, &cursor_hints))
                                } else {
                                    d.child(
                                        div()
                                            .font_family(fonts::MONO)
                                            .text_size(px(styles::META.size))
                                            .text_color(t.muted)
                                            .group_hover(group.clone(), |s| s.invisible())
                                            .child(when),
                                    )
                                    .child(self.hover_actions(ix, &group, cx))
                                }
                            }),
                    )
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        this.vm.cursor = ix;
                        let _ = &id;
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

    /// Archive, snooze and label buttons that take the time's place while the pointer is on a row.
    fn hover_actions(&self, ix: usize, group: &gpui::SharedString, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let mut acts = div()
            .absolute()
            .right_0()
            .top(px(-14.))
            .flex()
            .gap(px(2.))
            .invisible()
            .group_hover(group.clone(), |s| s.visible());
        for (i, (glyph, id)) in
            [("archive", sc::THREAD_ARCHIVE), ("clock", sc::THREAD_SNOOZE), ("tag", sc::THREAD_LABEL)]
                .into_iter()
                .enumerate()
        {
            acts = acts.child(
                div()
                    .id(("row-act", ix * 4 + i))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .hover(|s| s.bg(t.surface_strong))
                    .child(icon(glyph, 15., t.muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.vm.cursor = ix;
                        this.vm.selected.clear();
                        this.run(id, window, cx);
                    })),
            );
        }
        acts.into_any_element()
    }

    pub(crate) fn render_list(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        if self.vm.ids.is_empty() {
            return self.render_empty(cx);
        }
        let n = self.vm.ids.len();
        let _ = window;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .child(
                uniform_list("threads", n, cx.processor(|this, range, window, cx| this.render_rows(range, window, cx)))
                    .track_scroll(self.list_scroll.clone())
                    .flex_1()
                    .w_full(),
            )
            .into_any_element()
    }

    fn render_empty(&mut self, _cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let hint = |text: &str, ids: &[&str]| {
            let mut row = div().flex().items_center().gap(px(6.)).text_size(px(13.)).text_color(t.tertiary);
            for id in ids {
                row = row.child(keys(&t, &caps(id, km)));
            }
            row.child(text.to_owned())
        };
        let serif = |size: f32, text: String| {
            div()
                .font_family(fonts::SERIF)
                .text_size(px(size))
                .line_height(px(size * 1.15))
                .text_color(t.fg)
                .child(text)
        };
        let base = div().size_full().relative().flex().flex_col().items_center().justify_center().gap(px(10.)).bg(t.bg);
        match &self.vm.folder {
            Folder::Inbox if self.vm.account.is_none() => {
                // The earned dawn: the horizon rises once into place (drawn still with reduced motion).
                let reduce = self.reduce_motion;
                let horizon = div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(gpui::relative(0.3))
                    .child(super::widgets::horizon(&t, 1.));
                let horizon: AnyElement = if reduce {
                    horizon.into_any_element()
                } else {
                    horizon
                        .with_animation(
                            "dawn-rise",
                            gpui::Animation::new(std::time::Duration::from_millis(
                                akatsuki_tokens::motion::duration::SLOW,
                            ))
                            .with_easing(gpui::ease_out_quint()),
                            |el, p| el.bottom(gpui::relative(0.3 * p)).opacity(p),
                        )
                        .into_any_element()
                };
                base.overflow_hidden()
                    .child(horizon)
                    .child(serif(36., "Inbox zero".into()))
                    .child(hint("to undo the last one", &[sc::UNDO]))
                    .child(hint("for All mail", &[sc::GO_ALL]))
                    .into_any_element()
            }
            Folder::Search { query } => base
                .child(serif(24., format!("No results for “{query}”")))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(t.muted)
                        .child("Try fewer words, or search the full mailbox from the top bar."),
                )
                .into_any_element(),
            f => {
                let title = match f {
                    Folder::Snoozed => "Nothing snoozed",
                    Folder::Drafts => "No drafts",
                    Folder::Starred => "Nothing starred",
                    Folder::Reminders => "No reminders",
                    Folder::Spam => "No spam",
                    Folder::Trash => "Trash is empty",
                    Folder::Sent => "Nothing sent yet",
                    _ => "Nothing here",
                };
                base.child(serif(styles::EMPTY.size, title.into()))
                    .child(div().text_size(px(13.)).text_color(t.muted).child("This list is empty."))
                    .child(hint("for the Inbox", &[sc::GO_INBOX]))
                    .into_any_element()
            }
        }
    }
}
