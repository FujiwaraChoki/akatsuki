//! Colours, fonts and sizes for the GPUI chrome, derived only from `crates/tokens` (BRIEF "Design
//! theme": one accent, the foreground and accent opacity scales, Instrument Serif for headings and
//! Geist for dense UI). Nothing else in the app writes a colour literal.

use akatsuki_tokens::{Appearance, Palette, Rgb, TextStyle, font, opacity, text};
use gpui::{BoxShadow, Hsla, WindowAppearance, point, px, rgb, rgba};

/// The user's theme choice (`prefs.theme`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePref {
    /// Follow the OS.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl ThemePref {
    /// Resolves against the window's appearance.
    pub fn resolve(self, window: WindowAppearance) -> Appearance {
        match self {
            ThemePref::Light => Appearance::Light,
            ThemePref::Dark => Appearance::Dark,
            ThemePref::System => match window {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
                WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            },
        }
    }

    /// Label for settings and the palette.
    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "System",
            ThemePref::Light => "Light",
            ThemePref::Dark => "Dark",
        }
    }
}

fn solid(c: Rgb) -> Hsla {
    rgb(c.hex()).into()
}

fn alpha(c: Rgb, a: f32) -> Hsla {
    rgba(c.rgba_hex(a)).into()
}

/// The resolved colours for one appearance ("First Light", the design spec §2.3): ink on paper,
/// hairlines for structure, and red only where something needs the eye.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// The appearance.
    pub appearance: Appearance,
    /// Window background (paper).
    pub bg: Hsla,
    /// Text (ink).
    pub fg: Hsla,
    /// Secondary text (80 %).
    pub secondary: Hsla,
    /// Tertiary text (70 %): nav rows, text on surfaces.
    pub tertiary: Hsla,
    /// Muted text, placeholders, metadata (60 %).
    pub muted: Hsla,
    /// Strokes: input hover, active borders (40 %). Never text.
    pub stroke: Hsla,
    /// Strong hairline and input borders (20 %).
    pub line_strong: Hsla,
    /// Hairlines between regions (10 %).
    pub line: Hsla,
    /// The sidebar and status planes, hover rows, keycaps (4 %).
    pub surface: Hsla,
    /// Selected and active rows (7 %).
    pub surface_strong: Hsla,
    /// The emissive red: cursor bar, carets, recording dot, the horizon, the logo dot.
    pub dawn: Hsla,
    /// Accent fill, for the few brand buttons only.
    pub accent: Hsla,
    /// Text on an accent fill.
    pub on_accent: Hsla,
    /// Error text and destructive labels.
    pub accent_text: Hsla,
    /// Behind modal surfaces (palette, help).
    pub scrim: Hsla,
    /// Raw palette.
    pub palette: Palette,
}

impl Theme {
    /// The theme of an appearance.
    pub fn new(appearance: Appearance) -> Self {
        let p = appearance.palette();
        let over = |a: f32| solid(p.foreground.over(p.background, a));
        Self {
            appearance,
            bg: solid(p.background),
            fg: alpha(p.foreground, opacity::foreground::TEXT),
            secondary: over(opacity::foreground::SECONDARY),
            tertiary: over(opacity::foreground::TERTIARY),
            muted: over(opacity::foreground::MUTED),
            stroke: over(opacity::foreground::BORDER_HOVER),
            line_strong: over(opacity::foreground::BORDER),
            line: over(opacity::foreground::BORDER_SUBTLE),
            surface: over(opacity::foreground::SURFACE),
            surface_strong: over(opacity::foreground::SURFACE_STRONG),
            dawn: solid(p.dawn),
            accent: solid(p.accent),
            on_accent: solid(p.on_accent),
            accent_text: solid(p.accent_text),
            // Light: ink at 6 %; dark: paper at 50 % (§2.7). No blur.
            scrim: match appearance {
                Appearance::Light => alpha(p.foreground, 0.06),
                Appearance::Dark => alpha(p.background, 0.5),
            },
            palette: p,
        }
    }

    /// Level-1 elevation for everything raised (palette, pickers, toasts, the compose pop-out):
    /// one hairline (drawn by the caller) and one soft shadow, built from the tokens (§2.7).
    pub fn popover(&self) -> Vec<BoxShadow> {
        match self.appearance {
            Appearance::Light => vec![
                BoxShadow {
                    color: alpha(self.palette.foreground, akatsuki_tokens::effect::POP_OPACITY_LIGHT),
                    offset: point(px(0.), px(16.)),
                    blur_radius: px(48.),
                    spread_radius: px(-16.),
                },
                BoxShadow {
                    color: alpha(self.palette.foreground, 0.06),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(-2.),
                },
            ],
            Appearance::Dark => vec![BoxShadow {
                color: alpha(self.palette.background, akatsuki_tokens::effect::POP_OPACITY_DARK),
                offset: point(px(0.), px(16.)),
                blur_radius: px(48.),
                spread_radius: px(-16.),
            }],
        }
    }
}

/// Font family names.
pub mod fonts {
    /// Dense UI.
    pub const SANS: &str = super::font::sans::FAMILY;
    /// Brand, headings, opened subjects, empty states.
    pub const SERIF: &str = super::font::serif::FAMILY;
    /// Shortcuts and code.
    pub const MONO: &str = super::font::mono::FAMILY;
}

/// Text styles used by the chrome.
pub mod styles {
    use super::{TextStyle, text};
    /// 11 px captions.
    pub const CAPTION: TextStyle = text::app::CAPTION;
    /// 12 px metadata.
    pub const META: TextStyle = text::app::META;
    /// 13 px body.
    pub const BODY: TextStyle = text::app::BODY;
    /// 15 px reading text.
    pub const READING: TextStyle = text::app::READING;
    /// Mono caps: the reader crumb, the AI provenance line, chips (§2.1).
    pub const LABEL: TextStyle = text::app::LABEL;
    /// Section headers: 12/500, sentence case.
    pub const SECTION: TextStyle = text::app::SECTION;
    /// Opened-thread subject (serif).
    pub const SUBJECT: TextStyle = text::app::SUBJECT;
    /// Section headings (serif).
    pub const HEADING: TextStyle = text::app::HEADING;
    /// Empty states (serif).
    pub const EMPTY: TextStyle = text::app::EMPTY;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_follow_the_tokens() {
        let light = Theme::new(Appearance::Light);
        assert_eq!(light.accent, solid(akatsuki_tokens::LIGHT.accent));
        assert_eq!(light.bg, solid(akatsuki_tokens::LIGHT.background));
        assert_eq!(light.dawn, solid(akatsuki_tokens::LIGHT.dawn));
        let dark = Theme::new(Appearance::Dark);
        assert_eq!(dark.accent, solid(akatsuki_tokens::DARK.accent));
        assert_eq!(dark.bg, solid(akatsuki_tokens::DARK.background));
        assert_eq!(dark.on_accent, solid(akatsuki_tokens::DARK.on_accent));
        assert_eq!(dark.dawn, light.dawn, "the same red in both themes (D-431)");
        // Error text is the token's lifted crimson, never the deep fill colour (D-399).
        assert_eq!(dark.accent_text, solid(akatsuki_tokens::DARK.accent_text));
        assert_ne!(dark.accent_text, dark.accent);
        // Surfaces are the foreground over the paper: 4 % and 7 %, darker than the page in light mode.
        assert!(light.surface.l < light.bg.l && light.surface_strong.l < light.surface.l);
        assert!(dark.surface.l > dark.bg.l && dark.surface_strong.l > dark.surface.l);
        assert_eq!(dark.popover().len(), 1, "dark shadows are never lighter than the page");
    }

    #[test]
    fn theme_pref_resolves() {
        assert_eq!(ThemePref::System.resolve(WindowAppearance::Dark), Appearance::Dark);
        assert_eq!(ThemePref::Light.resolve(WindowAppearance::Dark), Appearance::Light);
        assert_eq!(ThemePref::Dark.resolve(WindowAppearance::Light), Appearance::Dark);
    }
}
