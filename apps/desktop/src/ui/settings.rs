//! Settings: appearance and keys (this device), BYO AI providers (profiles in Convex, keys in the
//! OS keychain, §12.2) and the account.

use gpui::{AnyElement, Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::convex::AiProviderKind;
use akatsuki_core::convex::rows::{AiModels, AiProfile};

use super::widgets::{button, field_frame, label, segment, switch};
use super::workspace::{SettingsTab, Workspace};
use crate::backend::SessionState;
use crate::config::{Density, KeymapPref, MotionPref};
use crate::theme::{ThemePref, fonts, styles};

/// Provider kinds offered in the form, with their labels.
pub const KINDS: [(AiProviderKind, &str); 8] = [
    (AiProviderKind::Openrouter, "OpenRouter"),
    (AiProviderKind::Anthropic, "Anthropic"),
    (AiProviderKind::Openai, "OpenAI"),
    (AiProviderKind::Google, "Google"),
    (AiProviderKind::Gateway, "AI Gateway"),
    (AiProviderKind::Ollama, "Ollama (local)"),
    (AiProviderKind::OpenaiCompatible, "OpenAI-compatible"),
    (AiProviderKind::Akatsuki, "Akatsuki (Pro)"),
];

fn row(t: &crate::theme::Theme, title: &str, detail: &str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .py(px(12.))
        .border_b_1()
        .border_color(t.border_subtle)
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().child(title.to_owned()))
                .when(!detail.is_empty(), |d| {
                    d.child(div().text_size(px(styles::META.size)).text_color(t.muted).child(detail.to_owned()))
                }),
        )
        .child(control)
}

impl Workspace {
    fn general(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let s = self.settings.clone();
        let mut themes = div().flex().gap(px(4.));
        for (i, th) in [ThemePref::System, ThemePref::Light, ThemePref::Dark].into_iter().enumerate() {
            themes = themes.child(segment(&t, ("theme", i), th.label(), s.theme == th).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.settings.theme = th;
                    this.save_settings();
                    this.apply_theme(window, cx);
                },
            )));
        }
        let mut density = div().flex().gap(px(4.));
        for (i, (d, name)) in
            [(Density::Comfortable, "Comfortable"), (Density::Compact, "Compact")].into_iter().enumerate()
        {
            density = density.child(segment(&t, ("density", i), name, s.density == d).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.density = d;
                    this.save_settings();
                    cx.notify();
                },
            )));
        }
        let mut keymaps = div().flex().gap(px(4.));
        for (i, (k, name)) in
            [(KeymapPref::Superhuman, "Superhuman"), (KeymapPref::Gmail, "Gmail")].into_iter().enumerate()
        {
            keymaps = keymaps.child(segment(&t, ("keymap", i), name, s.keymap == k).on_click(cx.listener(
                move |this, _, window, cx| {
                    let id = format!("{}{}", crate::commands::ids::KEYMAP, name.to_lowercase());
                    this.run(&id, window, cx);
                    let _ = k;
                },
            )));
        }
        let mut motion = div().flex().gap(px(4.));
        for (i, (m, name)) in
            [(MotionPref::System, "System"), (MotionPref::Reduce, "Reduce"), (MotionPref::Full, "Full")]
                .into_iter()
                .enumerate()
        {
            motion = motion.child(segment(&t, ("motion", i), name, s.motion == m).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.motion = m;
                    this.save_settings();
                    this.reduce_motion = match m {
                        MotionPref::System => this.system_reduces_motion,
                        MotionPref::Reduce => true,
                        MotionPref::Full => false,
                    };
                    cx.notify();
                },
            )));
        }
        let mut undo = div().flex().gap(px(4.));
        for (i, secs) in [0u32, 5, 10, 20, 30].into_iter().enumerate() {
            let text = if secs == 0 { "Off".to_owned() } else { format!("{secs} s") };
            undo = undo.child(segment(&t, ("undo", i), text, s.undo_send_seconds == secs).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.undo_send_seconds = secs;
                    this.save_settings();
                    cx.notify();
                },
            )));
        }
        let cache = s.cache_bodies;
        div()
            .flex()
            .flex_col()
            .child(row(&t, "Theme", "Light, dark, or follow the system.", themes))
            .child(row(&t, "Density", "Row height in lists.", density))
            .child(row(&t, "Keyboard shortcuts", "Superhuman keys, or Gmail's for snooze, back and read.", keymaps))
            .child(row(&t, "Motion", "The floating orbs and transitions.", motion))
            .child(row(&t, "Undo send", "How long a sent message waits so you can take it back.", undo))
            .child(row(
                &t,
                "Keep messages on this device",
                "Bodies are cached here (up to 2 GB) for instant, offline reading. Off keeps them in memory only.",
                switch(&t, "cache-bodies", cache).on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.cache_bodies = !cache;
                    this.backend.set_cache_bodies(!cache);
                    this.save_settings();
                    cx.notify();
                })),
            ))
            .into_any_element()
    }

    fn ai(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let settings = self.ai_settings.clone().unwrap_or_default();
        let consented = settings.consent.granted_at.is_some();
        let mut profiles = div().flex().flex_col().gap(px(8.));
        for (i, p) in settings.profiles.iter().enumerate() {
            let kind = KINDS.iter().find(|(k, _)| *k == p.kind).map_or("Provider", |(_, n)| n);
            let has_key = p.kind == AiProviderKind::Ollama
                || p.kind == AiProviderKind::Akatsuki
                || self.backend.has_ai_key(&p.profile_id);
            let egress = akatsuki_ai::settings::egress_text(p, akatsuki_core::convex::rows::AiRouteFeature::DEFAULT);
            let (test_p, remove_id) = (p.clone(), p.profile_id.clone());
            profiles = profiles.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .rounded(px(akatsuki_tokens::radius::MD))
                    .border_1()
                    .border_color(t.border)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(format!("{} · {kind}", p.label)))
                            .child(div().text_size(px(styles::META.size)).text_color(t.muted).child(egress))
                            .child(
                                div()
                                    .text_size(px(styles::META.size))
                                    .text_color(if has_key { t.muted } else { t.accent_text })
                                    .child(if has_key { "Key on this device" } else { "Key not set on this device" }),
                            ),
                    )
                    .child(button(&t, ("test", i), "Test connection", false).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.ai_form.status = Some("Testing…".into());
                            this.backend.test_ai_profile(test_p.clone());
                            cx.notify();
                        },
                    )))
                    .child(button(&t, ("remove", i), "Remove", false).on_click(cx.listener(move |this, _, _, cx| {
                        this.backend.remove_ai_profile(&remove_id);
                        cx.notify();
                    }))),
            );
        }
        let form = &self.ai_form;
        let mut kinds = div().flex().flex_wrap().gap(px(4.));
        for (i, (_, name)) in KINDS.iter().enumerate() {
            kinds = kinds.child(segment(&t, ("kind", i), *name, form.kind == i).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.ai_form.kind = i;
                    cx.notify();
                },
            )));
        }
        let field = |f: &gpui::Entity<super::input::TextField>, window: &Window, cx: &gpui::App| {
            field_frame(&t, f.focus_handle(cx).is_focused(window)).child(div().flex_1().child(f.clone()))
        };
        let consent = form.consent || consented;
        let status = form.status.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(div().text_color(t.secondary).child(
                "Bring your own AI provider. Keys stay in this computer's keychain and never reach Akatsuki's servers. \
                 With Ollama nothing leaves the machine.",
            ))
            .child(profiles)
            .child(div().pt(px(6.)).child(label(&t, "Add a provider")))
            .child(kinds)
            .child(field(&form.label, window, cx))
            .child(field(&form.base_url, window, cx))
            .child(field(&form.model, window, cx))
            .child(field(&form.key, window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(switch(&t, "ai-consent", consent).on_click(cx.listener(move |this, _, _, cx| {
                        this.ai_form.consent = !this.ai_form.consent;
                        cx.notify();
                    })))
                    .child(div().flex_1().text_size(px(styles::META.size)).text_color(t.secondary).child(
                        "I agree that AI features send the thread I run them on to the provider above (nothing is sent until I run one).",
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(button(&t, "ai-save", "Save provider", true).on_click(cx.listener(|this, _, _, cx| this.save_ai_form(cx))))
                    .when_some(status, |d, s| d.child(div().text_size(px(styles::META.size)).text_color(t.muted).child(s))),
            )
            .into_any_element()
    }

    fn save_ai_form(&mut self, cx: &mut Context<'_, Self>) {
        let f = &self.ai_form;
        let (kind, kind_name) = KINDS[f.kind.min(KINDS.len() - 1)];
        let label_text = f.label.read(cx).content.trim().to_owned();
        let base = f.base_url.read(cx).content.trim().to_owned();
        let model = f.model.read(cx).content.trim().to_owned();
        let key = f.key.read(cx).content.clone();
        if !base.is_empty()
            && let Some(problem) = akatsuki_ai::settings::base_url_problem(&base)
        {
            self.ai_form.status = Some(problem.to_owned());
            return cx.notify();
        }
        let profile = AiProfile {
            profile_id: uuid::Uuid::new_v4().to_string(),
            label: if label_text.is_empty() { kind_name.to_owned() } else { label_text },
            kind,
            // The managed provider has no base URL: the device's `AKATSUKI_API_URL` decides where it goes (D-415).
            base_url: (!base.is_empty() && kind != AiProviderKind::Akatsuki).then_some(base),
            models: AiModels {
                default: (!model.is_empty()).then(|| model.clone()),
                fast: (!model.is_empty()).then_some(model),
                ..AiModels::default()
            },
            key_on_devices: Vec::new(),
        };
        let consent = self.ai_form.consent;
        self.backend.save_ai_profile(profile, Some(key), consent);
        for e in [&self.ai_form.label, &self.ai_form.base_url, &self.ai_form.model, &self.ai_form.key] {
            e.update(cx, |f, cx| f.set_text("", cx));
        }
        self.ai_form.status = Some("Saved.".into());
        cx.notify();
    }

    fn account(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let demo = matches!(self.session, SessionState::SignedIn { demo: true, .. });
        div()
            .flex()
            .flex_col()
            .child(row(&t, "Signed in as", &self.email, div()))
            .child(row(&t, "This device", &format!("Desktop · {}", self.backend.device_id), div()))
            .child(row(&t, "Local data", &self.paths.data_dir.display().to_string(), div()))
            .child(row(
                &t,
                if demo { "Leave the demo" } else { "Sign out" },
                "Revokes this device's session and deletes its mirror, bodies, search index and AI cache.",
                button(&t, "sign-out", if demo { "Leave demo" } else { "Sign out" }, false)
                    .on_click(cx.listener(|this, _, window, cx| this.run(crate::commands::ids::SIGN_OUT, window, cx))),
            ))
            .into_any_element()
    }

    pub(crate) fn render_settings(
        &mut self,
        tab: SettingsTab,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = self.theme;
        let mut nav = div().w(px(180.)).flex().flex_col().gap(px(2.));
        for (i, (tb, name)) in
            [(SettingsTab::General, "General"), (SettingsTab::Ai, "AI providers"), (SettingsTab::Account, "Account")]
                .into_iter()
                .enumerate()
        {
            nav = nav.child(super::widgets::ghost(&t, ("settings-tab", i), tab == tb).child(name).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.overlay = super::workspace::Overlay::Settings(tb);
                    cx.notify();
                }),
            ));
        }
        let body = match tab {
            SettingsTab::General => self.general(cx),
            SettingsTab::Ai => self.ai(window, cx),
            SettingsTab::Account => self.account(cx),
        };
        div()
            .id("settings")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(t.bg)
            .occlude()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(64.))
                    .px(px(32.))
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(div().font_family(fonts::SERIF).text_size(px(styles::HEADING.size)).child("Settings"))
                    .child(div().flex_1())
                    .child(
                        button(&t, "settings-close", "Done", false)
                            .on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))),
                    ),
            )
            .child(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .flex()
                    .gap(px(32.))
                    .px(px(32.))
                    .py(px(24.))
                    .child(nav)
                    .child(div().flex_1().max_w(px(720.)).child(body)),
            )
            .into_any_element()
    }
}
