//! The 28 px status bar (§3.1.3): what the list shows (or an echo of the last action for 4 s), the
//! key HUD (the last three keys, gone a second later) and the sync state.

use gpui::{AnyElement, Context, div, prelude::*, px};

use akatsuki_core::time::now_ms;

use super::widgets::kbd;
use super::workspace::Workspace;
use crate::backend::SessionState;
use crate::theme::styles;

/// How long an action stays echoed.
const ECHO_MS: i64 = 4000;
/// How long a key stays in the HUD.
pub const HUD_MS: i64 = 1300;

impl Workspace {
    /// Records a key for the HUD (at most three are shown).
    pub(crate) fn hud_push(&mut self, key: String) {
        let now = now_ms();
        self.hud.retain(|(_, at)| now - at < HUD_MS);
        self.hud.push((key, now));
        if self.hud.len() > 3 {
            self.hud.remove(0);
        }
    }

    pub(crate) fn render_status(&mut self, _cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let now = now_ms();
        let echo = self.toasts.last().filter(|toast| now - toast.at < ECHO_MS).map(|toast| toast.text.clone());
        let n = self.vm.ids.len();
        let what = format!("{} · {} conversation{}", self.vm.folder.title(), n, if n == 1 { "" } else { "s" });
        let (left, left_color) = match echo {
            Some(e) => (e, t.secondary),
            None => (what, t.tertiary),
        };
        let waiting = self.store().outbox().map(|o| o.len()).unwrap_or(0);
        let accounts = self.store().accounts().map(|a| a.len()).unwrap_or(0);
        let demo = matches!(self.session, SessionState::SignedIn { demo: true, .. });
        let (sync, mark) = if waiting > 0 {
            (format!("Offline · {waiting} change{} waiting", if waiting == 1 { "" } else { "s" }), false)
        } else if !self.failures.is_empty() {
            (format!("{} failed · Retry from the palette", self.failures.len()), true)
        } else if demo {
            ("Demo · invented mail".to_owned(), false)
        } else {
            (format!("Synced · {accounts} account{}", if accounts == 1 { "" } else { "s" }), false)
        };
        let hud: Vec<String> = self.hud.iter().filter(|(_, at)| now - at < HUD_MS).map(|(k, _)| k.clone()).collect();
        div()
            .flex_none()
            .h(px(akatsuki_tokens::layout::STATUS_BAR_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(14.))
            .border_t_1()
            .border_color(t.line)
            .bg(t.surface)
            .text_size(px(styles::META.size))
            .text_color(t.tertiary)
            .child(div().flex_1().min_w(px(0.)).truncate().text_color(left_color).child(left))
            .child(div().flex().gap(px(3.)).children(hud.into_iter().map(|k| kbd(&t, k).bg(t.bg).text_color(t.fg))))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(if mark {
                        div().text_color(t.accent_text).child("!")
                    } else {
                        div().size(px(5.)).rounded_full().bg(t.muted)
                    })
                    .child(sync),
            )
            .into_any_element()
    }
}
