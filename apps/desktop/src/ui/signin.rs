//! The sign-in screen (§6.3; design §3.2): no card, a 360 px column under the serif headline
//! whose period is the dawn dot, the email code first (it works everywhere), magic link and social
//! sign-in through the browser and the loopback, the demo mailbox when no server is set up, and the
//! horizon drawn once at 72 % of the height.

use std::time::Duration;

use gpui::{Animation, AnimationExt, AnyElement, Context, Focusable, Window, div, prelude::*, px};

use super::widgets::{Kind, dot, field_frame, horizon, label, pill};
use super::workspace::Workspace;
use crate::auth::SocialProvider;
use crate::backend::SessionState;
use crate::theme::{fonts, styles};

impl Workspace {
    pub(crate) fn render_signin(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let height = f32::from(window.viewport_size().height);
        let configured = self.backend.endpoints().is_configured();
        let connecting = matches!(self.session, SessionState::Connecting);
        let reason = match &self.session {
            SessionState::SignedOut { reason } => reason.clone(),
            _ => None,
        };
        let status = self.signin.status.clone().or(reason);
        let email_focused = self.signin.email.focus_handle(cx).is_focused(window);
        let code_focused = self.signin.code.focus_handle(cx).is_focused(window);
        let size = 48.;
        let headline = div()
            .flex()
            .items_end()
            .font_family(fonts::SERIF)
            .text_size(px(size))
            .line_height(px(size))
            .child(if self.signin.code_sent { "Check your email" } else { "Mail at the speed of thought" })
            .when(!self.signin.code_sent, |d| d.child(dot(&t, size * 0.16).mb(px(size * 0.14)).ml(px(2.))));
        let lead = div().max_w(px(360.)).text_size(px(15.)).line_height(px(22.)).text_color(t.tertiary).child(
            if self.signin.code_sent {
                "We sent a 6-digit code. Type it here, or open the link in the email on this computer."
            } else {
                "One inbox for Gmail, Outlook and IMAP, driven from the keyboard."
            },
        );
        let mut form = div().w(px(360.)).flex().flex_col().gap(px(10.));
        let text_button = |id: &'static str, text: &'static str| {
            let fg = t.fg;
            div()
                .id(id)
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(t.tertiary)
                .hover(move |s| s.text_color(fg).underline())
                .child(text)
        };
        if connecting {
            form = form.child(div().text_color(t.muted).child("Connecting…"));
        } else if !configured {
            form = form
                .child(div().text_color(t.secondary).child(
                    "No server is set up on this computer. Self-hosting? Set AKATSUKI_CONVEX_URL and \
                     AKATSUKI_CONVEX_SITE_URL, or add them to desktop.json.",
                ))
                .child(
                    pill(&t, "demo", "Open the demo mailbox", Kind::Brand)
                        .w_full()
                        .h(px(40.))
                        .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::DEMO, window, cx))),
                )
                .child(
                    div()
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child("The demo uses invented mail and never connects to anything."),
                );
        } else {
            form = form
                .child(label(&t, "Email"))
                .child(field_frame(&t, email_focused).h(px(40.)).child(div().flex_1().child(self.signin.email.clone())))
                .when(self.signin.code_sent, |d| {
                    d.child(label(&t, "Code")).child(
                        field_frame(&t, code_focused)
                            .h(px(40.))
                            .font_family(fonts::MONO)
                            .text_size(px(18.))
                            .child(div().flex_1().child(self.signin.code.clone())),
                    )
                })
                .child(
                    pill(
                        &t,
                        "signin-primary",
                        if self.signin.code_sent { "Sign in" } else { "Email me a code" },
                        Kind::Brand,
                    )
                    .w_full()
                    .h(px(40.))
                    .text_size(px(14.))
                    .on_click(cx.listener(|this, _, window, cx| this.signin_primary(window, cx))),
                )
                .child(text_button("magic", "Send a sign-in link instead").on_click(cx.listener(|this, _, _, cx| {
                    let email = this.signin.email.read(cx).content.trim().to_owned();
                    if email.contains('@') {
                        this.backend.sign_in_browser(None, Some(email));
                    } else {
                        this.signin.status = Some("Enter your email address.".into());
                    }
                    cx.notify();
                })))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .py(px(4.))
                        .text_size(px(styles::META.size))
                        .text_color(t.muted)
                        .child(div().flex_1().h(px(1.)).bg(t.line))
                        .child("or")
                        .child(div().flex_1().h(px(1.)).bg(t.line)),
                )
                .children(
                    [SocialProvider::Google, SocialProvider::Microsoft, SocialProvider::Github]
                        .into_iter()
                        .enumerate()
                        .map(|(i, p)| {
                            pill(&t, ("social", i), p.label(), Kind::Secondary).w_full().h(px(40.)).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.backend.sign_in_browser(Some(p), None);
                                    cx.notify();
                                }),
                            )
                        }),
                )
                .child(
                    text_button("demo-link", "Or look around the demo mailbox")
                        .text_size(px(styles::META.size))
                        .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::DEMO, window, cx))),
                );
        }
        if let Some(s) = status {
            form = form.child(div().text_size(px(styles::META.size)).text_color(t.secondary).child(s));
        }
        // The horizon is drawn once from the centre outwards (§2.9: still with reduced motion). It
        // sits at 72 % of the height, or under the form when the form is taller.
        let line = div().w_full().flex().justify_center();
        let line: AnyElement = if self.reduce_motion {
            line.child(div().w_full().child(horizon(&t, 1.))).into_any_element()
        } else {
            line.child(
                div().w_full().child(horizon(&t, 1.)).with_animation(
                    "horizon-draw",
                    Animation::new(Duration::from_millis(akatsuki_tokens::motion::duration::SLOW))
                        .with_easing(gpui::ease_out_quint()),
                    |el, p| el.w(gpui::relative(p.max(0.02))),
                ),
            )
            .into_any_element()
        };
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(t.bg)
            .flex()
            .flex_col()
            .items_center()
            .child(div().absolute().top(px(24.)).left(px(24.)).child(dot(&t, 12.)))
            .child(div().flex_none().h(px((height * 0.16).max(32.))))
            .child(
                div()
                    .flex_none()
                    .w(px(520.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(14.))
                    .child(headline)
                    .child(lead)
                    .child(div().h(px(10.)))
                    .child(form),
            )
            .child(div().flex_1().min_h(px(40.)))
            .child(line)
            .child(
                div()
                    .flex_none()
                    .pt(px(16.))
                    .text_size(px(styles::META.size))
                    .text_color(t.muted)
                    .child("Open source, free for personal use. Message bodies and AI keys stay on this device."),
            )
            .child(div().flex_none().h(px(height * 0.2)))
            .into_any_element()
    }
}
