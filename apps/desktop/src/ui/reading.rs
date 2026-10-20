//! The reader (§3.1.5): crumb, the serif subject, the from line, the AI block with its provenance
//! line, messages without cards (collapsed 40 px rows, hairlines between), one-line notices and the
//! reply box. Remote images stay blocked; "Open original" shows the sanitized message in the browser.

use gpui::{AnyElement, Context, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::convex::AiFeature;
use akatsuki_shortcuts::ids as sc;

use super::body;
use super::widgets::{chip, crumb, icon, keys, pill};
use super::workspace::Workspace;
use crate::keys::caps;
use crate::theme::{fonts, styles};
use crate::vm::mail::Folder;
use crate::vm::time;

/// The short form of an egress sentence for the mono-caps provenance line (§3.8).
pub fn egress_short(sentence: &str) -> String {
    if sentence.starts_with("Stays on this device") {
        "On this device".into()
    } else if sentence.starts_with("Stays on your local network") {
        "On your network".into()
    } else if sentence.starts_with("Sent to OpenAI through Akatsuki") {
        "OpenAI via Akatsuki".into()
    } else if sentence.starts_with("Sent to Akatsuki") {
        "Akatsuki · not stored".into()
    } else if let Some(host) = sentence.rsplit_once("sent to ").map(|(_, h)| h.trim_end_matches('.')) {
        format!("Leaves this device · {host}")
    } else {
        "Leaves this device".into()
    }
}

impl Workspace {
    fn render_ai_panel(&mut self, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let open = self.vm.open.clone()?;
        let p = self.ai.as_ref().filter(|p| p.thread_id == open)?;
        let title = match p.feature {
            AiFeature::Summarize => "Summary",
            AiFeature::ExtractActions => "Action items",
            _ => "Suggested replies",
        };
        let mut provenance = vec![title.to_owned()];
        if let Some(m) = &p.model {
            provenance.push(m.rsplit('/').next().unwrap_or(m).to_owned());
        }
        if let Some(e) = &p.egress {
            provenance.push(egress_short(e));
        }
        let text = |s: String| div().text_size(px(13.)).line_height(px(20.)).text_color(t.secondary).child(s);
        let mut content = div().flex().flex_col().gap(px(4.)).min_h(px(40.));
        if let Some(e) = &p.error {
            content =
                content.child(div().text_size(px(styles::META.size)).text_color(t.accent_text).child(format!("! {e}")));
        } else if let Some(o) = &p.object {
            if let Some(tldr) = o.get("tldr").and_then(|v| v.as_str()) {
                content = content.child(text(tldr.to_owned()));
            }
            for key in ["bullets", "asks"] {
                if let Some(items) = o.get(key).and_then(|v| v.as_array()) {
                    let items: Vec<&str> = items.iter().filter_map(|v| v.as_str()).collect();
                    if key == "asks" && !items.is_empty() {
                        content = content.child(div().pt(px(6.)).child(crumb(&t, "Asks")));
                    }
                    for item in items {
                        content = content.child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(div().text_color(t.muted).child("–"))
                                .child(text(item.to_owned())),
                        );
                    }
                }
            }
            let actions = crate::vm::ai::action_items(o);
            if p.done && p.feature == AiFeature::ExtractActions && actions.is_empty() {
                content = content.child(div().text_color(t.muted).child("Nothing to do here."));
            }
            for (i, a) in actions.into_iter().enumerate() {
                let (title_text, due) = (a.title.clone(), a.due.clone());
                let thread = p.thread_id.clone();
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .py(px(4.))
                        .child(chip(&t, &a.kind))
                        .child(
                            div().flex_1().min_w(px(0.)).flex().flex_col().child(text(a.title.clone())).child(
                                div()
                                    .text_size(px(styles::META.size))
                                    .text_color(t.muted)
                                    .truncate()
                                    .child(format!("“{}”", a.quote)),
                            ),
                        )
                        .when_some(a.due.clone(), |d, due| {
                            d.child(div().font_family(fonts::MONO).text_size(px(12.)).text_color(t.muted).child(due))
                        })
                        .child(
                            pill(&t, ("ai-remind", i), "Remind me", super::widgets::Kind::Secondary)
                                .h(px(26.))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remind_action(&thread, &title_text, due.as_deref(), cx)
                                })),
                        ),
                );
            }
            if let Some(replies) = o.get("replies").and_then(|v| v.as_array()) {
                let mut row = div().flex().flex_wrap().gap(px(8.));
                for (i, r) in replies.iter().enumerate() {
                    let label_text = r.get("label").and_then(|v| v.as_str()).unwrap_or("Reply").to_owned();
                    let body_text = r.get("body").and_then(|v| v.as_str()).unwrap_or("").to_owned();
                    row = row.child(
                        pill(&t, ("ai-reply", i), label_text, super::widgets::Kind::Secondary)
                            .max_w(px(280.))
                            .text_color(t.secondary)
                            .font_weight(FontWeight::NORMAL)
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
                content = content.child(row);
            }
        } else if !p.text.is_empty() {
            content = content.child(text(p.text.clone()));
        } else {
            // Streaming: the 1 px dawn caret.
            content = content.child(div().w(px(1.)).h(px(14.)).bg(t.dawn));
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .px(px(16.))
                .py(px(14.))
                .rounded(px(akatsuki_tokens::radius::LG))
                .border_1()
                .border_color(t.line)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(div().flex_1().min_w(px(0.)).truncate().child(crumb(&t, &provenance.join(" · "))))
                        .child(
                            div()
                                .id("ai-hide")
                                .cursor_pointer()
                                .text_size(px(styles::META.size))
                                .text_color(t.muted)
                                .child("Hide")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.ai = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(content)
                .into_any_element(),
        )
    }

    /// The reader pane with nothing open (three-pane layout).
    pub(crate) fn render_no_thread(&mut self, _cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let hint = |ids: &[&str], what: &str| {
            let mut d = div().flex().items_center().gap(px(5.));
            for id in ids {
                d = d.child(keys(&t, &caps(id, km)));
            }
            d.child(what.to_owned())
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .bg(t.bg)
            .child(div().font_family(fonts::SERIF).text_size(px(24.)).child("Pick a conversation."))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(12.))
                    .text_size(px(13.))
                    .text_color(t.muted)
                    .child(hint(&[sc::NAV_NEXT, sc::NAV_PREV], "to move"))
                    .child(hint(&[sc::NAV_OPEN], "to open"))
                    .child(hint(&[sc::PALETTE_OPEN], "for everything else")),
            )
            .into_any_element()
    }

    pub(crate) fn render_reading(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
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
        let mut crumb_parts = vec![match &self.vm.folder {
            Folder::Search { .. } => "Search".to_owned(),
            f => f.title(),
        }];
        if self.vm.folder == Folder::Inbox
            && let Some(tab) = self.vm.tabs.iter().find(|x| Some(&x.key) == self.vm.tab.as_ref())
        {
            crumb_parts.push(tab.name.clone());
        }
        if let Some(c) = summary.as_ref().and_then(|s| s.triage.clone()) {
            crumb_parts.push(c);
        }
        let narrow = !self.three_panes(window);
        let now = akatsuki_core::time::now_ms();

        let mut messages = div().flex().flex_col();
        let mut from_line = String::new();
        if let Some(tv) = &self.thread {
            if let Some(last) = tv.messages.last() {
                let who =
                    last.from.as_ref().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                from_line = format!("{who} · to me · {}", time::list_time(last.date, now, &self.tz));
            }
            for (i, m) in tv.messages.iter().enumerate() {
                let mid = m.id.as_str().to_owned();
                let expanded = tv.expanded.contains(&mid);
                let from_name =
                    m.from.as_ref().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
                let when = time::list_time(m.date, now, &self.tz);
                if !expanded {
                    let id2 = mid.clone();
                    let surface = t.surface;
                    messages = messages.child(
                        div()
                            .id(("collapsed", i))
                            .flex_none()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .h(px(40.))
                            .border_b_1()
                            .border_color(t.line)
                            .cursor_pointer()
                            .hover(move |s| s.bg(surface))
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
                            .child(div().font_family(fonts::MONO).text_size(px(12.)).text_color(t.muted).child(when))
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
                let mut msg = div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .py(px(16.))
                    .when(i + 1 < tv.messages.len(), |d| d.border_b_1().border_color(t.line))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(from_name))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(styles::META.size))
                                    .text_color(t.muted)
                                    .child(format!("to {to}")),
                            )
                            .child(div().font_family(fonts::MONO).text_size(px(12.)).text_color(t.muted).child(when)),
                    );
                match tv.bodies.get(&mid) {
                    None => msg = msg.child(div().text_color(t.muted).child("Loading…")),
                    Some(Err(e)) => msg = msg.child(div().text_color(t.accent_text).child(format!("! {e}"))),
                    Some(Ok(r)) => {
                        // Notices are one tertiary line (§3.1.5).
                        let mut notice: Vec<String> = Vec::new();
                        if r.remote_images > 0 {
                            notice.push(format!(
                                "{} image{} hidden",
                                r.remote_images,
                                if r.remote_images == 1 { "" } else { "s" }
                            ));
                        }
                        if r.trackers > 0 {
                            notice.push(format!(
                                "{} tracker{} removed",
                                r.trackers,
                                if r.trackers == 1 { "" } else { "s" }
                            ));
                        }
                        if r.layout.complex {
                            notice.push("complex layout".into());
                        }
                        if !notice.is_empty() {
                            let fg = t.fg;
                            let (label_text, command) = if r.remote_images > 0 {
                                ("Show images in the original", sc::THREAD_SHOW_IMAGES)
                            } else {
                                ("Open the original", sc::THREAD_OPEN_ORIGINAL)
                            };
                            msg = msg.child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(styles::META.size))
                                    .text_color(t.tertiary)
                                    .child(notice.join(" · "))
                                    .child("·")
                                    .child(
                                        div()
                                            .id(("show-images", i))
                                            .cursor_pointer()
                                            .underline()
                                            .hover(move |s| s.text_color(fg))
                                            .child(label_text)
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| this.run(command, window, cx)),
                                            ),
                                    ),
                            );
                        }
                        let images = &r.inline_images;
                        msg = msg.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(10.))
                                .text_size(px(styles::READING.size))
                                .line_height(px(styles::READING.size * styles::READING.line_height))
                                .text_color(t.secondary)
                                .children(body::blocks(&t, &r.layout.blocks, &format!("m{i}"), images)),
                        );
                        if !r.layout.quoted.is_empty() {
                            let shown = tv.show_quoted.contains(&mid);
                            let id3 = mid.clone();
                            let fg = t.fg;
                            msg = msg.child(
                                div()
                                    .id(("quoted", i))
                                    .cursor_pointer()
                                    .text_size(px(styles::META.size))
                                    .text_color(t.muted)
                                    .hover(move |s| s.text_color(fg).underline())
                                    .child(if shown { "Hide quoted text" } else { "Show quoted text" })
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
                                msg = msg.child(
                                    div().flex().flex_col().gap(px(8.)).text_color(t.muted).children(body::blocks(
                                        &t,
                                        &r.layout.quoted,
                                        &format!("q{i}"),
                                        images,
                                    )),
                                );
                            }
                        }
                        if !r.attachments.is_empty() {
                            let mut row = div().flex().flex_wrap().gap(px(8.));
                            for a in &r.attachments {
                                row = row.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(6.))
                                        .h(px(28.))
                                        .px(px(10.))
                                        .rounded(px(akatsuki_tokens::radius::MD))
                                        .border_1()
                                        .border_color(t.line_strong)
                                        .text_size(px(styles::META.size))
                                        .child(icon("paperclip", 13., t.muted))
                                        .child(a.filename.clone())
                                        .child(div().text_color(t.muted).child(human_size(a.size))),
                                );
                            }
                            msg = msg.child(row);
                        }
                    }
                }
                messages = messages.child(msg);
            }
        }
        let ai = self.render_ai_panel(cx);
        let stroke = t.stroke;
        let reply_hints: Vec<(Vec<String>, &str)> = vec![
            (caps(sc::THREAD_REPLY_ALL, km), "all"),
            (caps(sc::THREAD_FORWARD, km), "forward"),
            (caps(sc::AI_INSTANT_REPLY, km), "suggest"),
        ];
        let reply_box = div()
            .id("reply-box")
            .group("reply-box")
            .flex_none()
            .h(px(40.))
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(14.))
            .pr(px(8.))
            .rounded(px(akatsuki_tokens::radius::LG))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .cursor_text()
            .hover(move |s| s.border_color(stroke))
            .child(div().flex_1().text_color(t.muted).child("Reply…"))
            .children(reply_hints.into_iter().map(|(c, what)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(styles::META.size))
                    .text_color(t.muted)
                    .invisible()
                    .group_hover("reply-box", |s| s.visible())
                    .child(keys(&t, &c))
                    .child(what)
            }))
            .child(keys(&t, &caps(sc::THREAD_REPLY, km)))
            .on_click(cx.listener(|this, _, window, cx| this.run(sc::THREAD_REPLY, window, cx)));
        div()
            .id("reading")
            .size_full()
            .bg(t.bg)
            .overflow_y_scroll()
            .track_scroll(&self.reading_scroll)
            .child(
                div()
                    .w_full()
                    .max_w(px(akatsuki_tokens::layout::CONTENT_MAX_WIDTH))
                    .when(narrow, |d| d.mx_auto())
                    .px(px(28.))
                    .pt(px(24.))
                    .pb(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(div().flex_1().child(crumb(&t, &crumb_parts.join(" · "))))
                            .child(
                                div()
                                    .id("open-original")
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .cursor_pointer()
                                    .text_size(px(styles::META.size))
                                    .text_color(t.muted)
                                    .child(icon("external", 13., t.muted))
                                    .child("Original")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.run(sc::THREAD_OPEN_ORIGINAL, window, cx)
                                    })),
                            )
                            .child(keys(&t, &caps(sc::NAV_BACK, km))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .font_family(fonts::SERIF)
                                    .text_size(px(styles::SUBJECT.size_wide.unwrap_or(styles::SUBJECT.size)))
                                    .line_height(px(styles::SUBJECT.size_wide.unwrap_or(styles::SUBJECT.size)
                                        * styles::SUBJECT.line_height))
                                    .child(subject),
                            )
                            .children(chips.iter().map(|c| chip(&t, c))),
                    )
                    .when(!from_line.is_empty(), |d| d.child(div().text_color(t.muted).child(from_line)))
                    .children(ai.map(|a| div().pt(px(8.)).child(a)))
                    .child(messages)
                    .child(div().pt(px(8.)).child(reply_box)),
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
    use super::*;

    #[test]
    fn sizes_read_well() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(3 * 1024 * 1024 / 2), "1.5 MB");
    }

    #[test]
    fn egress_has_a_short_form() {
        assert_eq!(egress_short("Stays on this device"), "On this device");
        assert_eq!(egress_short("Stays on your local network (nas.local)"), "On your network");
        assert_eq!(egress_short("Sent to Akatsuki for processing, not stored"), "Akatsuki · not stored");
        assert_eq!(
            egress_short("Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring"),
            "OpenAI via Akatsuki"
        );
        assert_eq!(
            egress_short("Leaves this device: the thread you run this on, sent to api.anthropic.com"),
            "Leaves this device · api.anthropic.com"
        );
    }
}
