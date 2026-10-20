//! Settings (design §3.10, the Linear pattern): a 216 px nav on the sidebar plane with "Back to
//! mail", a 640 px content column with a serif page title, and hairline groups of rows (title,
//! description, control). Appearance and keys (synced to `prefs`), BYO AI providers (profiles in
//! Convex, keys in the OS keychain, §12.2), labels, snippets and the account.

use gpui::{AnyElement, Context, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::convex::AiProviderKind;
use akatsuki_core::convex::rows::{AiModels, AiProfile};
use akatsuki_shortcuts::ids as sc;

use super::widgets::{Kind, field_frame, keys, pill, segment, segments, switch};
use super::workspace::{SettingsTab, Workspace};
use crate::backend::SessionState;
use crate::config::{Density, KeymapPref, MotionPref};
use crate::keys::caps;
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

/// One settings row: title 13/500 and a 12 px description on the left, the control on the right.
pub(super) fn row(t: &crate::theme::Theme, title: &str, detail: &str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(52.))
        .px(px(16.))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().font_weight(FontWeight::MEDIUM).child(title.to_owned()))
                .when(!detail.is_empty(), |d| {
                    d.child(div().text_size(px(styles::META.size)).text_color(t.muted).child(detail.to_owned()))
                }),
        )
        .child(control)
}

/// A hairline group of rows (radius lg, hairlines between rows).
pub(super) fn group(t: &crate::theme::Theme, rows: Vec<AnyElement>) -> gpui::Div {
    let mut g = div().flex().flex_col().rounded(px(akatsuki_tokens::radius::LG)).border_1().border_color(t.line);
    for (i, r) in rows.into_iter().enumerate() {
        g = g.child(div().when(i > 0, |d| d.border_t_1().border_color(t.line)).child(r));
    }
    g
}

/// A group header (13/500 ink, 32 above).
pub(super) fn group_title(text: &str) -> gpui::Div {
    div().pt(px(28.)).pb(px(8.)).font_weight(FontWeight::MEDIUM).child(text.to_owned())
}

impl Workspace {
    fn general(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let s = self.settings.clone();
        let mut themes = segments(&t);
        for (i, th) in [ThemePref::System, ThemePref::Light, ThemePref::Dark].into_iter().enumerate() {
            themes =
                themes.child(segment(&t, ("theme", i), th.label(), s.theme == th).on_click(
                    cx.listener(move |this, _, window, cx| this.change_settings(|s| s.theme = th, window, cx)),
                ));
        }
        let mut density = segments(&t);
        for (i, (d, name)) in
            [(Density::Comfortable, "Comfortable"), (Density::Compact, "Compact")].into_iter().enumerate()
        {
            density =
                density.child(segment(&t, ("density", i), name, s.density == d).on_click(
                    cx.listener(move |this, _, window, cx| this.change_settings(|s| s.density = d, window, cx)),
                ));
        }
        let mut keymaps = segments(&t);
        for (i, (k, name)) in
            [(KeymapPref::Superhuman, "Superhuman"), (KeymapPref::Gmail, "Gmail")].into_iter().enumerate()
        {
            keymaps = keymaps.child(segment(&t, ("keymap", i), name, s.keymap == k).on_click(cx.listener(
                move |this, _, window, cx| {
                    let id = format!("{}{}", crate::commands::ids::KEYMAP, name.to_lowercase());
                    this.run(&id, window, cx);
                },
            )));
        }
        let mut motion = segments(&t);
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
        let mut undo = segments(&t);
        for (i, secs) in [0u32, 5, 10, 20, 30].into_iter().enumerate() {
            let text = if secs == 0 { "Off".to_owned() } else { format!("{secs} s") };
            undo = undo.child(segment(&t, ("undo", i), text, s.undo_send_seconds == secs).on_click(cx.listener(
                move |this, _, window, cx| this.change_settings(|s| s.undo_send_seconds = secs, window, cx),
            )));
        }
        let cache = s.cache_bodies;
        div()
            .flex()
            .flex_col()
            .child(group_title("Appearance"))
            .child(group(
                &t,
                vec![
                    row(&t, "Theme", "Follows your system until you pick one.", themes).into_any_element(),
                    row(&t, "Density", "How tall rows are in lists.", density).into_any_element(),
                    row(&t, "Motion", "Overlay fades and the dawn. Reduce removes them.", motion).into_any_element(),
                ],
            ))
            .child(group_title("Keyboard and sending"))
            .child(group(
                &t,
                vec![
                    row(&t, "Keyboard shortcuts", "Superhuman keys, or Gmail's for snooze, back and read.", keymaps)
                        .into_any_element(),
                    row(&t, "Undo send", "How long a sent message waits so you can take it back.", undo)
                        .into_any_element(),
                ],
            ))
            .child(group_title("This device"))
            .child(group(
                &t,
                vec![
                    row(
                        &t,
                        "Keep messages on this device",
                        "Bodies are cached here (up to 2 GB) for instant, offline reading. Off keeps them in memory only.",
                        switch(&t, "cache-bodies", cache).on_click(cx.listener(move |this, _, window, cx| {
                            this.change_settings(|s| s.cache_bodies = !cache, window, cx)
                        })),
                    )
                    .into_any_element(),
                ],
            ))
            .into_any_element()
    }

    fn ai(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let settings = self.ai_settings.clone().unwrap_or_default();
        let consented = settings.consent.granted_at.is_some();
        let mut provider_rows: Vec<AnyElement> = Vec::new();
        for (i, p) in settings.profiles.iter().enumerate() {
            let kind = KINDS.iter().find(|(k, _)| *k == p.kind).map_or("Provider", |(_, n)| n);
            let has_key = p.kind == AiProviderKind::Ollama
                || p.kind == AiProviderKind::Akatsuki
                || self.backend.has_ai_key(&p.profile_id);
            let egress = akatsuki_ai::settings::egress_text(p, akatsuki_core::convex::rows::AiRouteFeature::DEFAULT);
            let host = p
                .base_url
                .as_deref()
                .and_then(|u| url::Url::parse(u).ok())
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_else(|| kind.to_owned());
            let (test_p, remove_id) = (p.clone(), p.profile_id.clone());
            let letter = p.label.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            provider_rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(akatsuki_tokens::radius::SM))
                            .border_1()
                            .border_color(t.line_strong)
                            .font_family(fonts::MONO)
                            .text_size(px(12.))
                            .child(letter),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(div().font_weight(FontWeight::MEDIUM).child(p.label.clone()))
                                    .child(div().text_size(px(styles::META.size)).text_color(t.muted).child(host)),
                            )
                            .child(div().text_size(px(styles::META.size)).text_color(t.tertiary).child(egress))
                            .child(
                                div()
                                    .text_size(px(styles::META.size))
                                    .text_color(if has_key { t.muted } else { t.accent_text })
                                    .child(if has_key { "Key on this device" } else { "Key not set on this device" }),
                            ),
                    )
                    .child(pill(&t, ("test", i), "Test connection", Kind::Secondary).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.ai_form.status = Some("Testing…".into());
                            this.backend.test_ai_profile(test_p.clone());
                            cx.notify();
                        },
                    )))
                    .child(pill(&t, ("remove", i), "Remove", Kind::Secondary).text_color(t.accent_text).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.backend.remove_ai_profile(&remove_id);
                            cx.notify();
                        }),
                    ))
                    .into_any_element(),
            );
        }
        if provider_rows.is_empty() {
            provider_rows.push(
                div()
                    .px(px(16.))
                    .py(px(14.))
                    .text_color(t.tertiary)
                    .child("No provider yet. Add one below, or use Ollama so nothing leaves this computer.")
                    .into_any_element(),
            );
        }
        let features = self.ai_features_rows(cx);
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
            .child(group_title("Providers"))
            .child(group(&t, provider_rows))
            .child(group_title("Features"))
            .child(features)
            .child(group_title("Add a provider"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .p(px(16.))
                    .rounded(px(akatsuki_tokens::radius::LG))
                    .border_1()
                    .border_color(t.line)
                    .child(div().text_size(px(styles::META.size)).text_color(t.tertiary).child(
                        "Keys stay in this computer's keychain and never reach Akatsuki's servers. With Ollama or a \
                         local endpoint nothing leaves the machine.",
                    ))
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
                            .child(
                                pill(&t, "ai-save", "Save provider", Kind::Ink)
                                    .on_click(cx.listener(|this, _, _, cx| this.save_ai_form(cx))),
                            )
                            .when_some(status, |d, s| {
                                d.child(div().text_size(px(styles::META.size)).text_color(t.muted).child(s))
                            }),
                    ),
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
            .child(group_title("Account"))
            .child(group(
                &t,
                vec![
                    row(&t, "Signed in as", &self.email, div()).into_any_element(),
                    row(&t, "This device", &format!("Desktop · {}", self.backend.device_id), div()).into_any_element(),
                    row(&t, "Local data", &self.paths.data_dir.display().to_string(), div()).into_any_element(),
                ],
            ))
            .child(group_title("Session"))
            .child(group(
                &t,
                vec![
                    row(
                        &t,
                        if demo { "Leave the demo" } else { "Sign out" },
                        "Revokes this device's session and deletes its mirror, bodies, search index and AI cache.",
                        pill(&t, "sign-out", if demo { "Leave demo" } else { "Sign out" }, Kind::Secondary)
                            .text_color(t.accent_text)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.run(crate::commands::ids::SIGN_OUT, window, cx)),
                            ),
                    )
                    .into_any_element(),
                ],
            ))
            .child(
                div()
                    .pt(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(styles::META.size))
                    .text_color(t.muted)
                    .child(super::widgets::dot(&t, 8.))
                    .child(div().font_family(fonts::SERIF).text_size(px(15.)).text_color(t.fg).child("Akatsuki"))
                    .child(format!("{} · Open source, free for personal use.", crate::config::APP_VERSION)),
            )
            .into_any_element()
    }

    pub(crate) fn render_settings(
        &mut self,
        tab: SettingsTab,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let t = self.theme;
        let km = self.settings.keymap.keymap();
        let pages = [
            (SettingsTab::General, "General", "Theme, density, keys and what stays on this device."),
            (SettingsTab::Ai, "AI providers", "Where AI runs, and what each feature sends."),
            (SettingsTab::Labels, "Labels", "Create, rename and delete labels in your mailboxes."),
            (SettingsTab::Snippets, "Snippets", "Text you insert with a key or a ;shortcut."),
            (SettingsTab::Account, "Account", "This device, its session and its data."),
        ];
        let mut nav = div()
            .w(px(akatsuki_tokens::layout::SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(1.))
            .px(px(8.))
            .bg(t.surface)
            .border_r_1()
            .border_color(t.line)
            .child(
                div()
                    .id("settings-back")
                    .h(px(akatsuki_tokens::layout::TOOLBAR_HEIGHT))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .cursor_pointer()
                    .text_color(t.tertiary)
                    .child(super::widgets::icon("arrow-left", 14., t.tertiary))
                    .child("Back to mail")
                    .child(keys(&t, &caps(sc::NAV_BACK, km)))
                    .on_click(cx.listener(|this, _, window, cx| this.close_overlay(window, cx))),
            );
        for (i, (tb, name, _)) in pages.iter().enumerate() {
            let tb = *tb;
            nav = nav.child(super::widgets::ghost(&t, ("settings-tab", i), tab == tb).child(*name).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.overlay = super::workspace::Overlay::Settings(tb);
                    cx.notify();
                }),
            ));
        }
        let (title, description) =
            pages.iter().find(|(tb, _, _)| *tb == tab).map(|(_, n, d)| (*n, *d)).unwrap_or(("Settings", ""));
        let body = match tab {
            SettingsTab::General => self.general(cx),
            SettingsTab::Ai => self.ai(window, cx),
            SettingsTab::Labels => self.labels_page(window, cx),
            SettingsTab::Snippets => self.snippets_page(window, cx),
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
            .child(nav)
            .child(
                div().id("settings-body").flex_1().min_w(px(0.)).h_full().overflow_y_scroll().child(
                    div()
                        .max_w(px(640. + 96.))
                        .px(px(48.))
                        .pt(px(40.))
                        .pb(px(64.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_family(fonts::SERIF)
                                .text_size(px(styles::HEADING.size))
                                .line_height(px(styles::HEADING.size * 1.2))
                                .child(title),
                        )
                        .child(div().pt(px(4.)).text_color(t.muted).child(description))
                        .child(body),
                ),
            )
            .into_any_element()
    }
}
