//! Everything that floats over the mail view: the command palette and pickers (with the crimson
//! orbs behind them), the shortcut sheet, settings, the compose panel, the undo-send bar and
//! toasts.

use std::time::Duration;

use gpui::{Animation, AnimationExt, AnyElement, Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::time::now_ms;
use akatsuki_shortcuts::{Platform, SHORTCUTS, ids as sc};

use super::widgets::{button, field_frame, icon, kbd, label};
use super::workspace::{Overlay, PickerKind, Workspace};
use crate::keys;
use crate::theme::{Theme, fonts, styles};
use crate::vm::compose::{AttachmentState, file_size};

/// Floating crimson orbs (BRIEF "Atmosphere"). Still when motion is reduced.
pub fn orbs(t: &Theme, reduce_motion: bool, n: usize) -> AnyElement {
    let seconds = akatsuki_tokens::motion::orb::FLOAT_SECONDS;
    let spots = [(0.12f32, 0.18f32, 280.0f32), (0.72, 0.62, 340.), (0.42, 0.85, 220.)];
    let mut layer = div().absolute().top_0().left_0().size_full().overflow_hidden();
    for (i, (x, y, size)) in spots.iter().copied().take(n).enumerate() {
        let orb = div()
            .absolute()
            .left(gpui::relative(x))
            .top(gpui::relative(y))
            .size(px(2.))
            .rounded_full()
            .shadow(t.orb(size));
        if reduce_motion {
            layer = layer.child(orb);
        } else {
            let secs = seconds.get(i).copied().unwrap_or(25.);
            layer = layer.child(orb.with_animation(
                ("orb", i),
                Animation::new(Duration::from_secs_f32(secs)).repeat(),
                move |el, delta| {
                    let a = delta * std::f32::consts::TAU;
                    el.left(gpui::relative(x + 0.06 * a.sin())).top(gpui::relative(y + 0.05 * (a * 1.3).cos()))
                },
            ));
        }
    }
    layer.into_any_element()
}

impl Workspace {
    fn modal(&self, child: impl IntoElement, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        div()
            .id("scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(t.scrim)
            .child(orbs(&t, self.reduce_motion, 2))
            .flex()
            .flex_col()
            .items_center()
            .pt(px(96.))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
            .child(child)
            .into_any_element()
    }

    fn render_palette(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let rows = self.palette_rows(cx);
        let sel = self.palette_sel.min(rows.len().saturating_sub(1));
        let title = match &self.overlay {
            Overlay::Picker(PickerKind::Snooze(_)) => Some("Snooze until"),
            Overlay::Picker(PickerKind::Remind(_)) => Some("Remind me if nobody replies by"),
            Overlay::Picker(PickerKind::Label(_)) => Some("Add label"),
            Overlay::Picker(PickerKind::Move(_)) => Some("Move to"),
            Overlay::Picker(PickerKind::GoLabel) => Some("Go to label"),
            Overlay::Picker(PickerKind::Unsubscribe { foreign: true, .. }) => {
                Some("Check the address: it is not at the sender's domain")
            }
            Overlay::Picker(PickerKind::Unsubscribe { .. }) => Some("Unsubscribe by email"),
            _ => None,
        };
        let mut list =
            div().id("palette-list").max_h(px(380.)).overflow_y_scroll().track_scroll(&self.palette_scroll).p(px(6.));
        let mut group = String::new();
        for (i, item) in rows.iter().enumerate() {
            if !item.group.is_empty() && item.group != group {
                group = item.group.clone();
                list = list
                    .child(div().px(px(10.)).pt(px(if i == 0 { 4. } else { 10. })).pb(px(4.)).child(label(&t, &group)));
            }
            let active = i == sel;
            let id = item.id.clone();
            list = list.child(
                div()
                    .id(("palette-row", i))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(34.))
                    .px(px(10.))
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .cursor_pointer()
                    .when(active, |d| d.bg(t.accent_tint))
                    .child(
                        div().w(px(14.)).flex_none().when(item.checked, |d| d.child(icon("check", 14., t.accent_text))),
                    )
                    .child(div().flex_1().min_w(px(0.)).truncate().child(item.label.clone()))
                    .when_some(item.hint.clone(), |d, h| d.child(kbd(&t, h)))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.palette_sel != i {
                            this.palette_sel = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_sel = i;
                        let _ = &id;
                        this.run(sc::PALETTE_RUN, window, cx);
                    })),
            );
        }
        if rows.is_empty() {
            list = list.child(div().py(px(18.)).flex().justify_center().text_color(t.muted).child("No matches"));
        }
        let platform = Platform::current();
        let hint = |id: &str| {
            akatsuki_shortcuts::find(id)
                .and_then(|s| {
                    keys::keys_for(s, platform, self.settings.keymap.keymap())
                        .first()
                        .map(|k| keys::format_keys(k, platform))
                })
                .unwrap_or_default()
        };
        let focused = self.palette_query.focus_handle(cx).is_focused(window);
        let panel = div()
            .id("palette")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(600.))
            .flex()
            .flex_col()
            .rounded(px(akatsuki_tokens::radius::LG))
            .bg(t.bg)
            .border_1()
            .border_color(t.border)
            .shadow(t.popover())
            .when_some(title, |d, title| {
                d.child(div().px(px(16.)).pt(px(14.)).font_family(fonts::SERIF).text_size(px(20.)).child(title))
            })
            .child(
                div().p(px(10.)).child(
                    field_frame(&t, focused)
                        .h(px(42.))
                        .child(icon("search", 15., t.muted))
                        .child(div().flex_1().child(self.palette_query.clone())),
                ),
            )
            .child(list)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .h(px(34.))
                    .px(px(14.))
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .text_size(px(styles::CAPTION.size))
                    .text_color(t.muted)
                    .child(div().flex().gap(px(5.)).child(kbd(&t, hint(sc::PALETTE_NEXT))).child("next"))
                    .child(div().flex().gap(px(5.)).child(kbd(&t, hint(sc::PALETTE_RUN))).child("run"))
                    .child(div().flex().gap(px(5.)).child(kbd(&t, hint(sc::PALETTE_CLOSE))).child("close")),
            );
        self.modal(panel, cx)
    }

    fn render_help(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let platform = Platform::current();
        let km = self.settings.keymap.keymap();
        let mut groups: Vec<(&str, Vec<(String, String)>)> = Vec::new();
        for s in SHORTCUTS {
            let keys_text: Vec<String> =
                keys::keys_for(s, platform, km).iter().map(|k| keys::format_keys(k, platform)).collect();
            if keys_text.is_empty() {
                continue;
            }
            match groups.iter_mut().find(|(g, _)| *g == s.group) {
                Some((_, v)) => v.push((s.label.to_owned(), keys_text.join("  or  "))),
                None => groups.push((s.group, vec![(s.label.to_owned(), keys_text.join("  or  "))])),
            }
        }
        let mut cols = div().flex().flex_wrap().gap(px(24.));
        for (g, items) in groups {
            let mut col = div().w(px(280.)).flex().flex_col().gap(px(6.)).child(label(&t, g));
            for (l, k) in items {
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().text_color(t.secondary).child(l))
                        .child(kbd(&t, k)),
                );
            }
            cols = cols.child(col);
        }
        let panel = div()
            .id("help")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(980.))
            .max_h(px(620.))
            .overflow_y_scroll()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .rounded(px(akatsuki_tokens::radius::LG))
            .bg(t.bg)
            .border_1()
            .border_color(t.border)
            .shadow(t.popover())
            .child(div().font_family(fonts::SERIF).text_size(px(styles::HEADING.size)).child("Keyboard shortcuts"))
            .child(cols);
        self.modal(panel, cx)
    }

    fn render_compose(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let c = self.compose.as_ref()?;
        let km = self.settings.keymap.keymap();
        let row = |name: &str, f: &gpui::Entity<super::input::TextField>, window: &Window, cx: &gpui::App| {
            let focused = f.focus_handle(cx).is_focused(window);
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(36.))
                .px(px(14.))
                .border_b_1()
                .border_color(if focused { t.accent_glow } else { t.border_subtle })
                .child(div().w(px(56.)).child(label(&t, name)))
                .child(div().flex_1().child(f.clone()))
        };
        let from = c.draft.identity_email.clone();
        let title = match &c.draft.kind {
            crate::vm::compose::ComposeKind::New => "New message",
            crate::vm::compose::ComposeKind::Reply { .. } => "Reply",
            crate::vm::compose::ComposeKind::Forward { .. } => "Forward",
        };
        let send_hint = keys::hint(sc::COMPOSE_SEND, km).unwrap_or_default();
        let cc_hint = keys::hint(sc::COMPOSE_CC, km).unwrap_or_default();
        let attach_hint = keys::hint(sc::COMPOSE_ATTACH, km).unwrap_or_default();
        // Attachment chips: name, size, upload state, remove.
        let chips = (!c.draft.attachments.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.)).px(px(14.)).pb(px(10.)).children(c.draft.attachments.iter().map(|a| {
                let key = a.key.clone();
                let (status, failed) = match &a.state {
                    AttachmentState::Uploading => ("Uploading…".to_owned(), false),
                    AttachmentState::Ready { .. } => (file_size(a.size), false),
                    AttachmentState::Failed(e) => (e.clone(), true),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .border_1()
                    .border_color(if failed { t.accent_glow } else { t.border_subtle })
                    .bg(t.bg)
                    .child(icon("paperclip", 13., t.muted))
                    .child(div().max_w(px(220.)).truncate().child(a.filename.clone()))
                    .child(
                        div()
                            .text_size(px(styles::META.size))
                            .text_color(if failed { t.link } else { t.muted })
                            .child(status),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from(format!("att-remove-{key}")))
                            .cursor_pointer()
                            .child(icon("close", 12., t.muted))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_attachment(&key, cx))),
                    )
            }))
        });
        let panel = div()
            .id("compose")
            .absolute()
            .right(px(24.))
            .bottom(px(24.))
            .w(px(620.))
            .flex()
            .flex_col()
            .rounded(px(akatsuki_tokens::radius::LG))
            .bg(t.bg)
            .border_1()
            .border_color(t.border)
            .shadow(t.popover())
            .occlude()
            // Files dropped anywhere on the composer are attached.
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(t.accent_glow).bg(t.accent_wash))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                this.attach_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(44.))
                    .px(px(14.))
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(div().font_family(fonts::SERIF).text_size(px(20.)).child(title))
                    .child(div().flex_1())
                    .child(div().text_size(px(styles::META.size)).text_color(t.muted).child(format!("from {from}")))
                    .child(
                        div()
                            .id("compose-close")
                            .ml(px(10.))
                            .cursor_pointer()
                            .child(icon("close", 15., t.muted))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_CLOSE, window, cx))),
                    ),
            )
            .child(row("To", &c.to, window, cx))
            .when(c.show_cc, |d| d.child(row("Cc", &c.cc, window, cx)).child(row("Bcc", &c.bcc, window, cx)))
            .child(row("Subject", &c.subject, window, cx))
            .child(
                div()
                    .px(px(14.))
                    .py(px(12.))
                    .min_h(px(220.))
                    .text_size(px(styles::READING.size))
                    .line_height(px(22.))
                    .child(c.body.clone()),
            )
            .children(chips)
            .when_some(c.error.clone(), |d, e| d.child(div().px(px(14.)).pb(px(6.)).text_color(t.link).child(e)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(52.))
                    .px(px(14.))
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .child(
                        button(&t, "send", "Send", true)
                            .child(div().ml(px(8.)).opacity(0.85).child(send_hint))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_SEND, window, cx))),
                    )
                    .child(
                        div()
                            .id("attach")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_color(t.muted)
                            .child(icon("paperclip", 14., t.muted))
                            .child(format!("Attach  {attach_hint}"))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_ATTACH, window, cx))),
                    )
                    .when(!c.show_cc, |d| {
                        d.child(
                            div()
                                .id("show-cc")
                                .cursor_pointer()
                                .text_color(t.muted)
                                .child(format!("Cc / Bcc  {cc_hint}"))
                                .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_CC, window, cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(div().min_w(px(0.)).truncate().text_size(px(styles::CAPTION.size)).text_color(t.faint).child(format!(
                        "Undo for {} s after sending",
                        self.settings.undo_send_seconds
                    ))),
            );
        Some(panel.into_any_element())
    }

    fn render_bottom(&mut self, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let now = now_ms();
        let km = self.settings.keymap.keymap();
        let undo_hint = keys::hint(sc::UNDO, km).unwrap_or_default();
        let mut stack =
            div().absolute().bottom(px(24.)).left_0().right_0().flex().flex_col().items_center().gap(px(8.));
        let mut any = false;
        if let Some(p) = &self.pending_send {
            any = true;
            let left = p.seconds_left(now);
            stack = stack.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(16.))
                    .h(px(40.))
                    .rounded_full()
                    .bg(t.fg)
                    .text_color(t.bg)
                    .shadow(t.popover())
                    .child(if left > 0 { format!("Sending in {left} s") } else { "Sending…".to_owned() })
                    .child(
                        div()
                            .id("undo-send")
                            .cursor_pointer()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Undo  {undo_hint}"))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::UNDO, window, cx))),
                    ),
            );
        }
        for toast in self.toasts.iter().rev().take(3) {
            any = true;
            stack = stack.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .px(px(16.))
                    .h(px(36.))
                    .rounded_full()
                    .bg(t.fg)
                    .text_color(t.bg)
                    .shadow(t.popover())
                    .text_size(px(styles::BODY.size))
                    .child(toast.text.clone())
                    .when(toast.undo, |d| {
                        d.child(
                            div()
                                .id(("toast-undo", toast.id as usize))
                                .cursor_pointer()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Undo  {undo_hint}"))
                                .on_click(cx.listener(|this, _, window, cx| this.run(sc::UNDO, window, cx))),
                        )
                    }),
            );
        }
        any.then(|| stack.into_any_element())
    }

    pub(crate) fn render_overlays(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.signed_in()
            && let Some(c) = self.render_compose(window, cx)
        {
            out.push(c);
        }
        match self.overlay.clone() {
            Overlay::Palette | Overlay::Picker(_) => out.push(self.render_palette(window, cx)),
            Overlay::Help => out.push(self.render_help(cx)),
            Overlay::Settings(tab) => out.push(self.render_settings(tab, window, cx)),
            Overlay::None => {}
        }
        if let Some(b) = self.render_bottom(cx) {
            out.push(b);
        }
        out
    }
}
