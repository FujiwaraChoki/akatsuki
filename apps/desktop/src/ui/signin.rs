//! The sign-in screen (§6.3): email code first (it works everywhere), magic link and social
//! sign-in through the browser and the loopback, or the demo mailbox when no server is set up.

use gpui::{AnyElement, Context, Focusable, Window, div, prelude::*, px};

use super::overlays::orbs;
use super::widgets::{button, field_frame, label};
use super::workspace::Workspace;
use crate::auth::SocialProvider;
use crate::backend::SessionState;
use crate::theme::{fonts, styles};

impl Workspace {
    pub(crate) fn render_signin(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let width = f32::from(window.viewport_size().width);
        let display = styles::DISPLAY;
        let size = display.size_for_width(width).min(96.);
        let configured = self.backend.endpoints().is_configured();
        let connecting = matches!(self.session, SessionState::Connecting);
        let reason = match &self.session {
            SessionState::SignedOut { reason } => reason.clone(),
            _ => None,
        };
        let status = self.signin.status.clone().or(reason);
        let email_focused = self.signin.email.focus_handle(cx).is_focused(window);
        let code_focused = self.signin.code.focus_handle(cx).is_focused(window);
        let mut card = div().w(px(380.)).flex().flex_col().gap(px(12.));
        if connecting {
            card = card.child(div().text_color(t.muted).child("Connecting…"));
        } else if !configured {
            card = card
                .child(div().text_color(t.secondary).child(
                    "No server is set up on this computer. Self-hosting? Set AKATSUKI_CONVEX_URL and \
                     AKATSUKI_CONVEX_SITE_URL, or add them to desktop.json.",
                ))
                .child(
                    button(&t, "demo", "Open the demo mailbox", true)
                        .w_full()
                        .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::DEMO, window, cx))),
                )
                .child(
                    div()
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child("The demo uses invented mail and never connects to anything."),
                );
        } else {
            card = card
                .child(label(&t, "Email"))
                .child(field_frame(&t, email_focused).h(px(40.)).child(div().flex_1().child(self.signin.email.clone())))
                .when(self.signin.code_sent, |d| {
                    d.child(label(&t, "Code")).child(
                        field_frame(&t, code_focused).h(px(40.)).child(div().flex_1().child(self.signin.code.clone())),
                    )
                })
                .child(
                    button(
                        &t,
                        "signin-primary",
                        if self.signin.code_sent { "Sign in" } else { "Email me a code" },
                        true,
                    )
                    .w_full()
                    .h(px(40.))
                    .on_click(cx.listener(|this, _, window, cx| this.signin_primary(window, cx))),
                )
                .child(
                    div()
                        .id("magic")
                        .cursor_pointer()
                        .text_size(px(styles::META.size))
                        .text_color(t.link)
                        .child("Send me a sign-in link instead")
                        .on_click(cx.listener(|this, _, _, cx| {
                            let email = this.signin.email.read(cx).content.trim().to_owned();
                            if email.contains('@') {
                                this.backend.sign_in_browser(None, Some(email));
                            } else {
                                this.signin.status = Some("Enter your email address.".into());
                            }
                            cx.notify();
                        })),
                )
                .child(div().h(px(1.)).bg(t.border_subtle).my(px(4.)))
                .children(
                    [SocialProvider::Google, SocialProvider::Microsoft, SocialProvider::Github]
                        .into_iter()
                        .enumerate()
                        .map(|(i, p)| {
                            button(&t, ("social", i), p.label(), false).w_full().h(px(38.)).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.backend.sign_in_browser(Some(p), None);
                                    cx.notify();
                                },
                            ))
                        }),
                )
                .child(
                    div()
                        .id("demo-link")
                        .cursor_pointer()
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child("Or look around the demo mailbox")
                        .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::DEMO, window, cx))),
                );
        }
        if let Some(s) = status {
            card = card.child(div().text_size(px(styles::META.size)).text_color(t.secondary).child(s));
        }
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(orbs(&t, self.reduce_motion, 3))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(akatsuki_tokens::space::S3))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(fonts::SERIF)
                                    .text_size(px(size))
                                    .line_height(px(size))
                                    .child("Akatsuki"),
                            )
                            .child(div().w(px(48.)).h(px(2.)).bg(t.accent))
                            .child(label(&t, "Dawn for your inbox")),
                    )
                    .child(card)
                    .child(
                        div().pt(px(12.)).text_size(px(styles::CAPTION.size)).text_color(t.faint).child(
                            "Open source, free for personal use. Message bodies and AI keys stay on this device.",
                        ),
                    ),
            )
            .into_any_element()
    }
}
