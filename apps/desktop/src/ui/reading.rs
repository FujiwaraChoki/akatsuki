//! The reading pane: the open thread, its messages rendered natively, the AI panel and the reply
//! bar. Remote images stay blocked; "Open original" shows the sanitized message in the browser.

use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::convex::AiFeature;
use akatsuki_shortcuts::ids as sc;

use super::body;
use super::widgets::{button, icon, kbd, label};
use super::workspace::Workspace;
use crate::theme::{fonts, styles};
use crate::vm::time;

impl Workspace {
    fn action_button(
        &self,
        i: usize,
        icon_name: &'static str,
        id: &'static str,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = self.theme;
        let hint = crate::keys::hint(id, self.settings.keymap.keymap());
        div()
            .id(("thread-action", i))
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .h(px(28.))
            .rounded(px(akatsuki_tokens::radius::MD))
            .cursor_pointer()
            .text_color(t.secondary)
            .hover(|s| s.bg(t.accent_wash))
            .child(icon(icon_name, 15., t.secondary))
            .when_some(hint, |d, h| d.child(div().text_size(px(styles::CAPTION.size)).text_color(t.faint).child(h)))
            .on_click(cx.listener(move |this, _, window, cx| this.run(id, window, cx)))
            .into_any_element()
    }

    fn render_ai_panel(&mut self, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let open = self.vm.open.clone()?;
        let p = self.ai.as_ref().filter(|p| p.thread_id == open)?;
        let title = match p.feature {
            AiFeature::Summarize => "Summary",
            _ => "Suggested replies",
        };
        let mut content = div().flex().flex_col().gap(px(6.));
        if let Some(e) = &p.error {
            content = content.child(div().text_color(t.link).child(e.clone()));
        } else if let Some(o) = &p.object {
            if let Some(tldr) = o.get("tldr").and_then(|v| v.as_str()) {
                content = content.child(
                    div()
                        .font_family(fonts::SERIF)
                        .text_size(px(akatsuki_tokens::text::brand::SUBHEAD.size))
                        .line_height(px(akatsuki_tokens::text::brand::SUBHEAD.size
                            * akatsuki_tokens::text::brand::SUBHEAD.line_height))
                        .child(tldr.to_owned()),
                );
            }
            for key in ["bullets", "asks"] {
                if let Some(items) = o.get(key).and_then(|v| v.as_array()) {
                    for item in items.iter().filter_map(|v| v.as_str()) {
                        let mark = if key == "asks" { "Ask" } else { "•" };
                        content = content.child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(div().text_color(t.muted).child(mark))
                                .child(item.to_owned()),
                        );
                    }
                }
            }
            if let Some(replies) = o.get("replies").and_then(|v| v.as_array()) {
                for (i, r) in replies.iter().enumerate() {
                    let label_text = r.get("label").and_then(|v| v.as_str()).unwrap_or("Reply").to_owned();
                    let body_text = r.get("body").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                    content = content.child(
                        div()
                            .id(("ai-reply", i))
                            .p(px(8.))
                            .rounded(px(akatsuki_tokens::radius::MD))
                            .border_1()
                            .border_color(t.border)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_wash))
                            .child(div().font_weight(FontWeight::MEDIUM).child(label_text))
                            .child(div().text_color(t.secondary).child(body_text.clone()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run(sc::THREAD_REPLY, window, cx);
                                if let Some(c) = &this.compose {
                                    let b = body_text.clone();
                                    c.body.update(cx, |f, cx| {
                                        let rest = f.content.clone();
                                        f.set_text(&format!("{b}{rest}"), cx);
                                    });
                                }
                            })),
                    );
                }
            }
        } else if !p.text.is_empty() {
            content = content.child(p.text.clone());
        } else {
            content = content.child(div().text_color(t.muted).child("Thinking…"));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(14.))
                .rounded(px(akatsuki_tokens::radius::LG))
                .bg(t.accent_wash)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("sparkle", 14., t.accent_text))
                        .child(label(&t, title)),
                )
                .child(content)
                .when_some(p.egress.clone(), |d, e| {
                    d.child(div().text_size(px(styles::CAPTION.size)).text_color(t.muted).child(e))
                })
                .into_any_element(),
        )
    }

    pub(crate) fn render_reading(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let Some(open) = self.vm.open.clone() else { return div().into_any_element() };
        let thread = self.store().thread(&open).ok().flatten();
        let labels = self.store().labels(None).unwrap_or_default();
        let summary = self.store().summary(&open).ok().flatten();
        let subject = thread
            .as_ref()
            .map(|t| t.subject.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "(no subject)".into());
        let chips: Vec<String> = summary
            .as_ref()
            .map(|s| {
                s.label_keys
                    .iter()
                    .filter_map(|k| {
                        labels.iter().find(|l| &l.label_key == k && l.role.is_none()).map(|l| l.name.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let actions = [
            ("arrow-left", sc::NAV_BACK),
            ("archive", sc::THREAD_ARCHIVE),
            ("trash", sc::THREAD_TRASH),
            ("clock", sc::THREAD_SNOOZE),
            ("star", sc::THREAD_STAR),
            ("tag", sc::THREAD_LABEL),
            ("sparkle", sc::AI_SUMMARIZE),
        ];
        let mut bar = div().flex().items_center().gap(px(2.));
        for (i, (ic, id)) in actions.iter().enumerate() {
            bar = bar.child(self.action_button(i, ic, id, cx));
        }
        bar = bar.child(div().flex_1()).child(
            div()
                .id("open-original")
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .h(px(28.))
                .rounded(px(akatsuki_tokens::radius::MD))
                .cursor_pointer()
                .text_size(px(styles::META.size))
                .text_color(t.secondary)
                .hover(|s| s.bg(t.accent_wash))
                .child(icon("external", 14., t.secondary))
                .child("Open original")
                .on_click(cx.listener(|this, _, window, cx| this.run(sc::THREAD_OPEN_ORIGINAL, window, cx))),
        );

        let mut messages = div().flex().flex_col().gap(px(10.));
        if let Some(tv) = &self.thread {
            let n = tv.messages.len();
            for (i, m) in tv.messages.iter().enumerate() {
                let mid = m.id.as_str().to_owned();
                let expanded = tv.expanded.contains(&mid);
                let from_name =
                    m.from.as_ref().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                let from_email = m.from.as_ref().map(|a| a.email.clone()).unwrap_or_default();
                let when = time::full_time(m.date, &self.tz);
                if !expanded {
                    let id2 = mid.clone();
                    messages = messages.child(
                        div()
                            .id(("collapsed", i))
                            // Fixed height: a collapsed row must never shrink under its text in the column.
                            .flex_none()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(14.))
                            .h(px(40.))
                            .rounded(px(akatsuki_tokens::radius::MD))
                            .border_1()
                            .border_color(t.border_subtle)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.accent_wash))
                            .child(
                                div()
                                    .w(px(160.))
                                    .flex_none()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(from_name),
                            )
                            .child(
                                div().flex_1().min_w(px(0.)).truncate().text_color(t.muted).child(one_line(&m.snippet)),
                            )
                            .child(div().text_size(px(styles::META.size)).text_color(t.muted).child(when))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(tv) = &mut this.thread {
                                    tv.expanded.insert(id2.clone());
                                }
                                if this.backend.cached_body(&id2).is_none() {
                                    this.backend.load_body(&id2);
                                }
                                cx.notify();
                            })),
                    );
                    continue;
                }
                let to =
                    m.to.iter()
                        .map(|a| a.name.clone().unwrap_or_else(|| a.email.clone()))
                        .collect::<Vec<_>>()
                        .join(", ");
                let mut card = div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(18.))
                    .rounded(px(akatsuki_tokens::radius::LG))
                    .border_1()
                    .border_color(t.border_subtle)
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap(px(8.))
                                            .child(div().font_weight(FontWeight::SEMIBOLD).child(from_name))
                                            .child(div().text_color(t.muted).child(from_email)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(styles::META.size))
                                            .text_color(t.muted)
                                            .child(format!("to {to}")),
                                    ),
                            )
                            .child(div().text_size(px(styles::META.size)).text_color(t.muted).child(when)),
                    );
                match tv.bodies.get(&mid) {
                    None => card = card.child(div().text_color(t.muted).child("Loading…")),
                    Some(Err(e)) => card = card.child(div().text_color(t.link).child(e.clone())),
                    Some(Ok(r)) => {
                        if r.layout.complex {
                            card = card.child(
                                div()
                                    .id(("complex", i))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .p(px(8.))
                                    .rounded(px(akatsuki_tokens::radius::MD))
                                    .bg(t.accent_wash)
                                    .text_size(px(styles::META.size))
                                    .cursor_pointer()
                                    .child(icon("info", 14., t.accent_text))
                                    .child("This message has a complex layout. Open the original to see it as sent.")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.run(sc::THREAD_OPEN_ORIGINAL, window, cx)
                                    })),
                            );
                        }
                        let images = &r.inline_images;
                        card = card.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(10.))
                                .text_size(px(styles::READING.size))
                                .line_height(px(styles::READING.size * styles::READING.line_height))
                                .children(body::blocks(&t, &r.layout.blocks, &format!("m{i}"), images)),
                        );
                        if !r.layout.quoted.is_empty() {
                            let shown = tv.show_quoted.contains(&mid);
                            let id3 = mid.clone();
                            card = card.child(
                                div()
                                    .id(("quoted", i))
                                    .w(px(36.))
                                    .h(px(18.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(akatsuki_tokens::radius::SM))
                                    .bg(t.accent_wash)
                                    .cursor_pointer()
                                    .text_color(t.muted)
                                    .child("•••")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(tv) = &mut this.thread
                                            && !tv.show_quoted.remove(&id3)
                                        {
                                            tv.show_quoted.insert(id3.clone());
                                        }
                                        cx.notify();
                                    })),
                            );
                            if shown {
                                card = card.child(
                                    div().flex().flex_col().gap(px(8.)).text_color(t.secondary).children(body::blocks(
                                        &t,
                                        &r.layout.quoted,
                                        &format!("q{i}"),
                                        images,
                                    )),
                                );
                            }
                        }
                        if r.remote_images > 0 {
                            let hint = crate::keys::hint(sc::THREAD_SHOW_IMAGES, self.settings.keymap.keymap());
                            card = card.child(
                                div()
                                    .id(("images", i))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_size(px(styles::META.size))
                                    .text_color(t.muted)
                                    .child(icon("image", 13., t.muted))
                                    .child(format!(
                                        "{} remote image{} blocked{}.",
                                        r.remote_images,
                                        if r.remote_images == 1 { "" } else { "s" },
                                        if r.trackers > 0 {
                                            format!(
                                                ", {} tracker{} removed",
                                                r.trackers,
                                                if r.trackers == 1 { "" } else { "s" }
                                            )
                                        } else {
                                            String::new()
                                        }
                                    ))
                                    .child(
                                        div()
                                            .id(("show-images", i))
                                            .cursor_pointer()
                                            .text_color(t.link)
                                            .child("Show images in the original")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.run(sc::THREAD_SHOW_IMAGES, window, cx)
                                            })),
                                    )
                                    .when_some(hint, |d, h| d.child(kbd(&t, h))),
                            );
                        }
                        if !r.attachments.is_empty() {
                            let mut row = div().flex().flex_wrap().gap(px(8.));
                            for a in &r.attachments {
                                row = row.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .px(px(10.))
                                        .py(px(6.))
                                        .rounded(px(akatsuki_tokens::radius::MD))
                                        .border_1()
                                        .border_color(t.border)
                                        .text_size(px(styles::META.size))
                                        .child(icon("paperclip", 13., t.muted))
                                        .child(a.filename.clone())
                                        .child(div().text_color(t.muted).child(human_size(a.size))),
                                );
                            }
                            card = card.child(row);
                        }
                    }
                }
                messages = messages.child(card);
                let _ = n;
            }
        }
        let km = self.settings.keymap.keymap();
        let reply_bar = div()
            .flex()
            .gap(px(8.))
            .child(
                button(&t, "reply", "Reply", true)
                    .when_some(crate::keys::hint(sc::THREAD_REPLY, km), |d, h| {
                        d.child(div().ml(px(8.)).opacity(0.8).child(h))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.run(sc::THREAD_REPLY, window, cx))),
            )
            .child(
                button(&t, "reply-all", "Reply all", false)
                    .when_some(crate::keys::hint(sc::THREAD_REPLY_ALL, km), |d, h| {
                        d.child(div().ml(px(8.)).text_color(t.muted).child(h))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.run(sc::THREAD_REPLY_ALL, window, cx))),
            )
            .child(
                button(&t, "forward", "Forward", false)
                    .when_some(crate::keys::hint(sc::THREAD_FORWARD, km), |d, h| {
                        d.child(div().ml(px(8.)).text_color(t.muted).child(h))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.run(sc::THREAD_FORWARD, window, cx))),
            )
            .child(
                button(&t, "instant", "Instant reply", false)
                    .when_some(crate::keys::hint(sc::AI_INSTANT_REPLY, km), |d, h| {
                        d.child(div().ml(px(8.)).text_color(t.muted).child(h))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.run(sc::AI_INSTANT_REPLY, window, cx))),
            );
        let ai = self.render_ai_panel(cx);
        div()
            .id("reading")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.reading_scroll)
            .child(
                div()
                    .mx_auto()
                    .max_w(px(akatsuki_tokens::layout::CONTENT_MAX_WIDTH + 64.))
                    .w_full()
                    .px(px(akatsuki_tokens::space::S4))
                    .py(px(akatsuki_tokens::space::S2))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(bar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(fonts::SERIF)
                                    .text_size(px(styles::SUBJECT.size + 6.))
                                    .line_height(px(36.))
                                    .child(subject),
                            )
                            .when(!chips.is_empty(), |d| {
                                d.child(div().flex().gap(px(6.)).children(chips.into_iter().map(|c| {
                                    div()
                                        .px(px(8.))
                                        .py(px(2.))
                                        .rounded_full()
                                        .bg(t.accent_wash)
                                        .text_size(px(styles::CAPTION.size))
                                        .text_color(t.secondary)
                                        .child(c)
                                })))
                            }),
                    )
                    .children(ai)
                    .child(messages)
                    .child(reply_bar)
                    .child(div().h(px(48.))),
            )
            .into_any_element()
    }
}

/// Collapses whitespace (snippets may carry line breaks).
pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_read_well() {
        assert_eq!(super::human_size(512), "512 B");
        assert_eq!(super::human_size(2048), "2 KB");
        assert_eq!(super::human_size(3 * 1024 * 1024 / 2), "1.5 MB");
    }
}
