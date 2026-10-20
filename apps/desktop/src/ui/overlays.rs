//! Everything that floats over the mail view (level-1 elevation, no blur, §2.7): the command
//! palette (§3.5), pickers as popovers anchored to what they act on (§3.7), the shortcut sheet
//! (§3.11), the composer (in the reader slot from 1100 px, else a pop-out; §3.4), the undo-send
//! pill and toasts (bottom-left of the list, §3.9).

use std::time::Duration;

use gpui::{Animation, AnimationExt, AnyElement, Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::time::now_ms;
use akatsuki_shortcuts::{Platform, SHORTCUTS, ids as sc};

use super::widgets::{Kind, field_frame, icon, kbd, keys, label, pill, raised};
use super::workspace::{Overlay, PickerKind, Workspace};
use crate::keys::{self, caps};
use crate::theme::{fonts, styles};
use crate::vm::compose::{AttachmentState, ComposeKind, file_size};

/// Where a picker sits.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    /// Under the cursor row of the list.
    Row(f32),
    /// Above the composer's footer.
    Compose,
    /// Centred like the palette.
    Centre,
}

/// The overlay fade (§2.9: 140 ms in, nothing when motion is reduced).
fn appear<E: IntoElement + gpui::Styled + 'static>(el: E, id: &'static str, reduce: bool) -> AnyElement {
    if reduce {
        return el.into_any_element();
    }
    el.with_animation(
        id,
        Animation::new(Duration::from_millis(akatsuki_tokens::motion::duration::QUICK))
            .with_easing(gpui::ease_out_quint()),
        |el, p| el.opacity(p),
    )
    .into_any_element()
}

impl Workspace {
    fn scrim(&self, child: AnyElement, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        div()
            .id("scrim")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(t.scrim)
            .flex()
            .flex_col()
            .items_center()
            .pt(gpui::relative(0.12))
            .on_mouse_down(gpui::MouseButton::Left, cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
            .child(child)
            .into_any_element()
    }

    /// Where the open picker anchors.
    fn anchor(&self, window: &Window) -> Anchor {
        match &self.overlay {
            Overlay::Picker(PickerKind::SendLater | PickerKind::Snippet | PickerKind::Rewrite) => Anchor::Compose,
            Overlay::Picker(
                PickerKind::Snooze(_) | PickerKind::Remind(_) | PickerKind::Label(_) | PickerKind::Move(_),
            ) if self.vm.open.is_none() || self.three_panes(window) => {
                let row_h = self.settings.density.row_height();
                let scroll = f32::from(self.list_scroll.0.borrow().base_handle.offset().y);
                let y = akatsuki_tokens::layout::TOOLBAR_HEIGHT + (self.vm.cursor as f32 + 1.) * row_h + scroll + 4.;
                let max = f32::from(window.viewport_size().height) - 360.;
                Anchor::Row(y.clamp(akatsuki_tokens::layout::TOOLBAR_HEIGHT, max.max(60.)))
            }
            _ => Anchor::Centre,
        }
    }

    fn render_palette(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let rows = self.palette_rows(cx);
        let sel = self.palette_sel.min(rows.len().saturating_sub(1));
        let picker = matches!(self.overlay, Overlay::Picker(_));
        let title = match &self.overlay {
            Overlay::Picker(PickerKind::Snooze(_)) => Some("Snooze until"),
            Overlay::Picker(PickerKind::Remind(_)) => Some("Remind me if nobody replies by"),
            Overlay::Picker(PickerKind::Label(_)) => Some("Label"),
            Overlay::Picker(PickerKind::Move(_)) => Some("Move to"),
            Overlay::Picker(PickerKind::GoLabel) => Some("Go to label"),
            Overlay::Picker(PickerKind::Unsubscribe { foreign: true, .. }) => {
                Some("Check the address: it is not at the sender's domain")
            }
            Overlay::Picker(PickerKind::Unsubscribe { .. }) => Some("Unsubscribe by email"),
            Overlay::Picker(PickerKind::SendLater) => Some("Send later"),
            Overlay::Picker(PickerKind::Reschedule(_)) => Some("Send at"),
            Overlay::Picker(PickerKind::Snippet) => Some("Insert snippet"),
            Overlay::Picker(PickerKind::Rewrite) => Some("Rewrite with AI"),
            Overlay::Picker(PickerKind::Scheduled) => Some("Scheduled messages"),
            Overlay::Picker(PickerKind::ScheduledAction(_)) => Some("This message"),
            Overlay::Picker(PickerKind::LabelCreate) => Some("Create label"),
            Overlay::Picker(PickerKind::LabelRename) => Some("Rename label"),
            Overlay::Picker(PickerKind::LabelRenameTo { .. }) => Some("New name"),
            Overlay::Picker(PickerKind::LabelRemove | PickerKind::LabelRemoveConfirm { .. }) => Some("Delete label"),
            Overlay::Picker(PickerKind::ContinueDraft) => Some("Continue a draft"),
            Overlay::Picker(PickerKind::AiSearch) => Some("Search with AI"),
            _ => None,
        };
        let anchor = if picker { self.anchor(window) } else { Anchor::Centre };
        let row_h = if picker { 32. } else { 40. };
        let width = match (&self.overlay, anchor) {
            (Overlay::Picker(PickerKind::Snippet), _) => 400.,
            (Overlay::Picker(PickerKind::Label(_) | PickerKind::Move(_)), _) => 320.,
            (Overlay::Picker(_), Anchor::Row(_) | Anchor::Compose) => 280.,
            _ => 560.,
        };
        let mut list = div()
            .id("palette-list")
            .max_h(px(if picker { 320. } else { 380. }))
            .overflow_y_scroll()
            .track_scroll(&self.palette_scroll)
            .p(px(4.));
        let mut group = String::new();
        for (i, item) in rows.iter().enumerate() {
            if !item.group.is_empty() && item.group != group {
                group = item.group.clone();
                list = list.child(div().h(px(28.)).flex().items_end().px(px(12.)).pb(px(4.)).child(label(&t, &group)));
            }
            let active = i == sel;
            let hint_caps: Option<Vec<String>> = (!picker)
                .then(|| akatsuki_shortcuts::find(&item.id).map(|_| caps(&item.id, km)).filter(|c| !c.is_empty()))
                .flatten();
            let surface_strong = t.surface_strong;
            list = list.child(
                div()
                    .id(("palette-row", i))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(row_h))
                    .px(px(if picker { 10. } else { 12. }))
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .cursor_pointer()
                    .text_size(px(if picker { 13. } else { 14. }))
                    .text_color(if active { t.fg } else { t.secondary })
                    .when(active, move |d| d.bg(surface_strong))
                    .when(item.checked, |d| d.child(icon("check", 13., t.fg)))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(item.label.clone()))
                    .when_some(item.hint.clone().filter(|_| hint_caps.is_none()), |d, h| {
                        d.child(
                            div()
                                .flex_none()
                                .font_family(if picker { fonts::MONO } else { fonts::SANS })
                                .text_size(px(styles::CAPTION.size))
                                .text_color(t.muted)
                                .child(h),
                        )
                    })
                    .when_some(hint_caps, |d, c| d.child(keys(&t, &c)))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.palette_sel != i {
                            this.palette_sel = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_sel = i;
                        this.run(sc::PALETTE_RUN, window, cx);
                    })),
            );
        }
        if rows.is_empty() {
            list = list.child(div().py(px(14.)).px(px(12.)).text_color(t.muted).child("No matches"));
        }
        let footer_hint = |id: &str, what: &str| {
            div().flex().items_center().gap(px(5.)).child(keys(&t, &caps(id, km))).child(what.to_owned())
        };
        let focused = self.palette_query.focus_handle(cx).is_focused(window);
        let input = if picker {
            div()
                .px(px(8.))
                .pt(px(4.))
                .child(field_frame(&t, focused).h(px(32.)).child(div().flex_1().child(self.palette_query.clone())))
        } else {
            div()
                .h(px(52.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(18.))
                .border_b_1()
                .border_color(t.line)
                .text_size(px(16.))
                .child(icon("search", 15., t.muted))
                .child(div().flex_1().child(self.palette_query.clone()))
        };
        let panel = raised(&t)
            .id("palette")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(width))
            .flex()
            .flex_col()
            .overflow_hidden()
            .when_some(title, |d, title| {
                d.child(div().px(px(10.)).pt(px(8.)).pb(px(2.)).child(label(&t, title).px(px(2.))))
            })
            .child(input)
            .child(list)
            .when(!picker, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .h(px(32.))
                        .px(px(14.))
                        .border_t_1()
                        .border_color(t.line)
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child(footer_hint(sc::PALETTE_NEXT, "move"))
                        .child(footer_hint(sc::PALETTE_RUN, "run"))
                        .child(footer_hint(sc::PALETTE_CLOSE, "close")),
                )
            });
        let reduce = self.reduce_motion;
        match anchor {
            Anchor::Centre => {
                let el = appear(panel, "palette-in", reduce);
                self.scrim(el, cx)
            }
            // Anchored popovers have no scrim (§2.7); a click outside closes them.
            Anchor::Row(y) => div()
                .id("popover-catch")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
                )
                .child(
                    div().absolute().top(px(y)).left(px(akatsuki_tokens::layout::SIDEBAR_WIDTH + 40.)).child(appear(
                        panel,
                        "popover-in",
                        reduce,
                    )),
                )
                .into_any_element(),
            Anchor::Compose => div()
                .id("popover-catch")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .bottom(px(akatsuki_tokens::layout::STATUS_BAR_HEIGHT + 64.))
                        .right(px(32.))
                        .child(appear(panel, "popover-in", reduce)),
                )
                .into_any_element(),
        }
    }

    fn render_help(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let platform = Platform::current();
        let km = self.settings.keymap.keymap();
        let mut groups: Vec<(&str, Vec<(String, Vec<String>)>)> = Vec::new();
        for s in SHORTCUTS {
            let Some(first) = keys::keys_for(s, platform, km).first() else { continue };
            let c = keys::keycaps(first, platform);
            match groups.iter_mut().find(|(g, _)| *g == s.group) {
                Some((_, v)) => v.push((s.label.to_owned(), c)),
                None => groups.push((s.group, vec![(s.label.to_owned(), c)])),
            }
        }
        let mut cols = div().flex().flex_wrap().gap(px(28.));
        for (g, items) in groups {
            let mut col = div().w(px(254.)).flex().flex_col().child(div().h(px(28.)).child(label(&t, g)));
            for (l, k) in items {
                col = col.child(
                    div()
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w(px(0.)).truncate().text_color(t.secondary).child(l))
                        .child(keys(&t, &k)),
                );
            }
            cols = cols.child(col);
        }
        let panel = raised(&t)
            .id("help")
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(880.))
            .max_h(px(640.))
            .overflow_y_scroll()
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).child("Keyboard shortcuts"))
                    .child(keys(&t, &caps(sc::NAV_BACK, km))),
            )
            .child(cols);
        let el = appear(panel, "help-in", self.reduce_motion);
        self.scrim(el, cx)
    }

    /// The composer's content (§3.4): header, recipient rows, the body, AI strip, attachments and
    /// the footer with the ink Send pill.
    fn compose_panel(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<gpui::Stateful<gpui::Div>> {
        let t = self.theme;
        let ai_strip = self.render_compose_ai(window, cx);
        let tools = self.render_compose_tools(cx);
        let c = self.compose.as_ref()?;
        let km = self.settings.keymap.keymap();
        let row = |name: &str, f: &gpui::Entity<super::input::TextField>, window: &Window, cx: &gpui::App| {
            let focused = f.focus_handle(cx).is_focused(window);
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(36.))
                .border_b_1()
                .border_color(if focused { t.stroke } else { t.line })
                .child(div().w(px(56.)).flex_none().text_color(t.muted).child(name.to_owned()))
                .child(div().flex_1().child(f.clone()))
        };
        let from = c.draft.identity_email.clone();
        let title = match &c.draft.kind {
            ComposeKind::New => "New message",
            ComposeKind::Reply { .. } => "Reply",
            ComposeKind::Forward { .. } => "Forward",
        };
        let saved = c.draft.base_version > 0;
        // Attachment chips: name, size, upload state, remove.
        let chips = (!c.draft.attachments.is_empty()).then(|| {
            div().flex().flex_wrap().gap(px(6.)).pb(px(10.)).children(c.draft.attachments.iter().map(|a| {
                let key = a.key.clone();
                let (status, failed) = match &a.state {
                    AttachmentState::Uploading => ("Uploading…".to_owned(), false),
                    AttachmentState::Ready { .. } => (file_size(a.size), false),
                    AttachmentState::Failed { reason } => (reason.clone(), true),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(10.))
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .border_1()
                    .border_color(if failed { t.accent_text } else { t.line_strong })
                    .bg(t.bg)
                    .child(icon("paperclip", 13., t.muted))
                    .child(div().max_w(px(220.)).truncate().child(a.filename.clone()))
                    .child(
                        div()
                            .text_size(px(styles::META.size))
                            .text_color(if failed { t.accent_text } else { t.muted })
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
        let stroke = t.stroke;
        let panel = div()
            .id("compose")
            .flex()
            .flex_col()
            .px(px(24.))
            .pt(px(14.))
            .pb(px(12.))
            .bg(t.bg)
            .occlude()
            // Files dropped anywhere on the composer are attached.
            .drag_over::<gpui::ExternalPaths>(move |style, _, _, _| style.border_color(stroke))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                this.attach_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pb(px(8.))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(styles::META.size))
                            .text_color(t.muted)
                            .child(if saved { format!("Draft saved · from {from}") } else { format!("from {from}") }),
                    )
                    .child(
                        div()
                            .id("compose-close")
                            .cursor_pointer()
                            .child(keys(&t, &caps(sc::COMPOSE_CLOSE, km)))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_CLOSE, window, cx))),
                    ),
            )
            .child(row("To", &c.to, window, cx).when(!c.show_cc, |d| {
                d.child(
                    div()
                        .id("show-cc")
                        .cursor_pointer()
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child("Cc Bcc")
                        .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_CC, window, cx))),
                )
            }))
            .when(c.show_cc, |d| d.child(row("Cc", &c.cc, window, cx)).child(row("Bcc", &c.bcc, window, cx)))
            .child(row("Subject", &c.subject, window, cx))
            .child(
                div()
                    .flex_1()
                    .py(px(12.))
                    .min_h(px(200.))
                    .text_size(px(styles::READING.size))
                    .line_height(px(styles::READING.size * styles::READING.line_height))
                    .child(c.body.clone()),
            )
            .children(chips)
            .children(ai_strip)
            .when_some(c.error.clone(), |d, e| {
                d.child(div().pb(px(6.)).text_size(px(styles::META.size)).text_color(t.accent_text).child(format!("! {e}")))
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        pill(&t, "send", "Send", Kind::Ink)
                            .h(px(30.))
                            .pr(px(6.))
                            .child(
                                div().flex().gap(px(3.)).children(caps(sc::COMPOSE_SEND, km).into_iter().map(|k| {
                                    kbd(&t, k).bg(t.fg).border_color(t.secondary).text_color(t.bg)
                                })),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_SEND, window, cx))),
                    )
                    .child(
                        pill(&t, "later", "Send later", Kind::Secondary)
                            .h(px(30.))
                            .pr(px(6.))
                            .child(keys(&t, &caps(sc::COMPOSE_SEND_LATER, km)))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_SEND_LATER, window, cx))),
                    )
                    .child(tools)
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("attach")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(akatsuki_tokens::radius::MD))
                            .cursor_pointer()
                            .hover(|s| s.bg(t.surface))
                            .child(icon("paperclip", 15., t.muted))
                            .on_click(cx.listener(|this, _, window, cx| this.run(sc::COMPOSE_ATTACH, window, cx))),
                    ),
            );
        Some(panel)
    }

    /// The composer in the reader slot (three panes).
    pub(crate) fn render_compose_slot(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        match self.compose_panel(window, cx) {
            Some(p) => {
                let el = p.size_full();
                if self.reduce_motion {
                    el.into_any_element()
                } else {
                    el.with_animation(
                        "compose-in",
                        Animation::new(Duration::from_millis(160)).with_easing(gpui::ease_out_quint()),
                        |el, p| el.opacity(p).pl(px(24. + 16. * (1. - p))),
                    )
                    .into_any_element()
                }
            }
            None => div().into_any_element(),
        }
    }

    /// The pop-out composer (narrow windows).
    fn render_compose_popout(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let panel = self.compose_panel(window, cx)?;
        Some(
            div()
                .absolute()
                .right(px(24.))
                .bottom(px(akatsuki_tokens::layout::STATUS_BAR_HEIGHT + 16.))
                .w(px(620.))
                .h(px(560.))
                .child(
                    panel
                        .size_full()
                        .border_1()
                        .border_color(t.line_strong)
                        .rounded(px(akatsuki_tokens::radius::LG))
                        .shadow(t.popover()),
                )
                .into_any_element(),
        )
    }

    fn render_bottom(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let now = now_ms();
        let km = self.settings.keymap.keymap();
        let undo_caps = caps(sc::UNDO, km);
        let ink_kbd = |k: String| kbd(&t, k).bg(t.fg).border_color(t.secondary).text_color(t.bg);
        let pill_box = || {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .pl(px(14.))
                .pr(px(6.))
                .h(px(36.))
                .rounded_full()
                .bg(t.fg)
                .text_color(t.bg)
                .shadow(t.popover())
                .text_size(px(styles::BODY.size))
        };
        // Bottom-left of the list pane, 12 px above the status bar; never over the reader.
        let mut stack = div()
            .absolute()
            .bottom(px(akatsuki_tokens::layout::STATUS_BAR_HEIGHT + 12.))
            .left(px(akatsuki_tokens::layout::SIDEBAR_WIDTH + 16.))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.));
        let mut any = false;
        if let Some(p) = &self.pending_send {
            any = true;
            let left = p.seconds_left(now);
            stack = stack.child(
                pill_box().child(if left > 0 { format!("Sending in {left} s") } else { "Sending…".to_owned() }).child(
                    div()
                        .id("undo-send")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Undo")
                        .children(undo_caps.iter().cloned().map(ink_kbd))
                        .on_click(cx.listener(|this, _, window, cx| this.run(sc::UNDO, window, cx))),
                ),
            );
        }
        // At most one info toast, errors (with actions) above it.
        let errors: Vec<_> = self.toasts.iter().filter(|t| !t.actions.is_empty()).rev().take(2).cloned().collect();
        let info = self.toasts.iter().rev().find(|t| t.actions.is_empty()).cloned();
        for toast in errors.into_iter().rev().chain(info) {
            any = true;
            let error = !toast.actions.is_empty();
            stack = stack.child(
                pill_box()
                    .max_w(px(640.))
                    .when(error, |d| {
                        d.child(
                            div()
                                .size(px(14.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(t.bg)
                                .text_size(px(10.))
                                .child("!"),
                        )
                    })
                    .child(div().min_w(px(0.)).truncate().child(toast.text.clone()))
                    .children(toast.actions.iter().enumerate().map(|(i, (text, command))| {
                        let command = command.clone();
                        div()
                            .id(gpui::SharedString::from(format!("toast-{}-{i}", toast.id)))
                            .flex_none()
                            .cursor_pointer()
                            .font_weight(FontWeight::MEDIUM)
                            .child(text.clone())
                            .on_click(cx.listener(move |this, _, window, cx| this.run(&command, window, cx)))
                    }))
                    .when(toast.undo, |d| {
                        d.child(
                            div()
                                .id(("toast-undo", toast.id as usize))
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Undo")
                                .children(undo_caps.iter().cloned().map(ink_kbd))
                                .on_click(cx.listener(|this, _, window, cx| this.run(sc::UNDO, window, cx))),
                        )
                    })
                    .when(!toast.undo && !error, |d| d.pr(px(14.))),
            );
        }
        let _ = window;
        any.then(|| appear(stack, "toast-in", self.reduce_motion))
    }

    pub(crate) fn render_overlays(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if self.signed_in()
            && !self.three_panes(window)
            && let Some(c) = self.render_compose_popout(window, cx)
        {
            out.push(c);
        }
        match self.overlay.clone() {
            Overlay::Palette | Overlay::Picker(_) => out.push(self.render_palette(window, cx)),
            Overlay::Help => out.push(self.render_help(cx)),
            Overlay::Settings(tab) => out.push(self.render_settings(tab, window, cx)),
            Overlay::None => {}
        }
        if self.signed_in()
            && let Some(b) = self.render_bottom(window, cx)
        {
            out.push(b);
        }
        out
    }

    /// The composer's AI strip: the "write with AI" prompt row, a running request and dictation,
    /// each with the egress line of its feature (§12.2, §3.8).
    fn render_compose_ai(&self, window: &mut Window, cx: &mut Context<'_, Self>) -> Option<AnyElement> {
        let t = self.theme;
        let c = self.compose.as_ref()?;
        let note = |text: String| div().text_size(px(styles::CAPTION.size)).text_color(t.muted).child(text);
        let mut strip = div().flex().flex_col().gap(px(6.)).pb(px(8.));
        let mut any = false;
        if let Some(edit) = &c.ai_edit {
            any = true;
            let what = if edit.feature == akatsuki_core::convex::AiFeature::Rewrite { "Rewriting" } else { "Writing" };
            strip = strip
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(styles::META.size))
                        .text_color(t.tertiary)
                        .child(div().w(px(1.)).h(px(14.)).bg(t.dawn))
                        .child(format!("{what}… Esc stops and keeps your text")),
                )
                .children(self.backend.ai_egress(edit.feature).map(note));
        } else if c.ai_bar {
            any = true;
            let focused = c.instruction.focus_handle(cx).is_focused(window);
            strip = strip.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        field_frame(&t, focused).flex_1().h(px(36.)).child(div().flex_1().child(c.instruction.clone())),
                    )
                    .child(
                        pill(&t, "ai-write", "Write", Kind::Ink)
                            .h(px(30.))
                            .on_click(cx.listener(|this, _, window, cx| this.run_ai_write(window, cx))),
                    ),
            );
            strip = strip.child(note(
                self.backend
                    .ai_egress(akatsuki_core::convex::AiFeature::Compose)
                    .unwrap_or_else(|| "Set up an AI provider in Settings first".into()),
            ));
        }
        if let Some(d) = &c.dictation {
            any = true;
            let (line, egress, recording) = match d {
                crate::backend::DictationState::Recording { started_ms, egress } => {
                    let secs = (now_ms() - started_ms).max(0) / 1000;
                    (format!("Listening… {}:{:02} · Dictate again to stop", secs / 60, secs % 60), egress.clone(), true)
                }
                crate::backend::DictationState::Transcribing { egress } => {
                    ("Transcribing…".to_owned(), egress.clone(), false)
                }
                _ => (String::new(), String::new(), false),
            };
            strip = strip
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(styles::META.size))
                        .text_color(t.tertiary)
                        .when(recording, |d| d.child(div().size(px(6.)).rounded_full().bg(t.dawn)))
                        .child(line),
                )
                .when(!egress.is_empty(), |d| d.child(note(egress)));
        }
        if c.ai_undo.is_some() && c.ai_edit.is_none() {
            any = true;
            let fg = t.fg;
            strip = strip.child(
                div()
                    .id("ai-undo")
                    .cursor_pointer()
                    .text_size(px(styles::META.size))
                    .text_color(t.muted)
                    .underline()
                    .hover(move |s| s.text_color(fg))
                    .child("Undo the AI change")
                    .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::AI_UNDO, window, cx))),
            );
        }
        any.then(|| strip.into_any_element())
    }

    /// Ghost buttons beside Send: snippet, write with AI, rewrite, dictate.
    fn render_compose_tools(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let recording = self.backend.is_recording();
        let tool = |id: &'static str, text: &str, command: &'static str| {
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
                .text_color(t.tertiary)
                .hover(move |s| s.bg(surface))
                .when(command == sc::COMPOSE_DICTATE && recording, |d| {
                    d.child(div().size(px(6.)).rounded_full().bg(t.dawn))
                })
                .child(text.to_owned())
                .child(keys(&t, &caps(command, km)))
                .on_click(cx.listener(move |this, _, window, cx| this.run(command, window, cx)))
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(2.))
            .child(tool("tool-ai", "Write with AI", sc::COMPOSE_AI_WRITE))
            .child(tool("tool-snippet", "Snippet", sc::COMPOSE_SNIPPET))
            .child(tool("tool-rewrite", "Rewrite", sc::COMPOSE_REWRITE))
            .child(tool("tool-dictate", if recording { "Stop" } else { "Dictate" }, sc::COMPOSE_DICTATE))
            .into_any_element()
    }
}
