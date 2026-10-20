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

/// The resolved colours for one appearance.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// Window background.
    pub bg: Hsla,
    /// Text.
    pub fg: Hsla,
    /// Secondary text (80 %).
    pub secondary: Hsla,
    /// Muted text, placeholders, metadata (60 %).
    pub muted: Hsla,
    /// Hovered borders and disabled text (40 %).
    pub faint: Hsla,
    /// Borders (20 %).
    pub border: Hsla,
    /// Hairlines between regions (10 %).
    pub border_subtle: Hsla,
    /// The accent.
    pub accent: Hsla,
    /// Accent at 70 % (gradient core).
    pub accent_core: Hsla,
    /// Accent at 30 % (glow).
    pub accent_glow: Hsla,
    /// Accent at 15 % (selected rows, text selection).
    pub accent_tint: Hsla,
    /// Accent at 8 % (hover, cursor row).
    pub accent_wash: Hsla,
    /// Text on an accent fill.
    pub on_accent: Hsla,
    /// Accent-coloured text, icons and thin strokes: the `accentText` token (the crimson in light
    /// mode, the deep crimson lifted toward the foreground in dark mode, D-399). Fills keep `accent`.
    pub accent_text: Hsla,
    /// Link text: `accent_text`.
    pub link: Hsla,
    /// Behind modal surfaces.
    pub scrim: Hsla,
    /// Raw palette.
    pub palette: Palette,
}

impl Theme {
    /// The theme of an appearance.
    pub fn new(appearance: Appearance) -> Self {
        let p = appearance.palette();
        let fg = opacity::foreground::TEXT;
        Self {
            bg: solid(p.background),
            fg: alpha(p.foreground, fg),
            secondary: solid(p.foreground.over(p.background, opacity::foreground::SECONDARY)),
            muted: solid(p.foreground.over(p.background, opacity::foreground::MUTED)),
            faint: solid(p.foreground.over(p.background, opacity::foreground::BORDER_HOVER)),
            border: solid(p.foreground.over(p.background, opacity::foreground::BORDER)),
            border_subtle: solid(p.foreground.over(p.background, opacity::foreground::BORDER_SUBTLE)),
            accent: solid(p.accent),
            accent_core: alpha(p.accent, opacity::accent::CORE),
            accent_glow: alpha(p.accent, opacity::accent::GLOW),
            accent_tint: alpha(p.accent, opacity::accent::TINT),
            accent_wash: alpha(p.accent, opacity::accent::WASH),
            // White on crimson in light mode; the light foreground on deep crimson in dark mode.
            on_accent: match appearance {
                Appearance::Light => solid(p.background),
                Appearance::Dark => solid(p.foreground),
            },
            accent_text: solid(p.accent_text),
            link: solid(p.accent_text),
            scrim: alpha(p.background, opacity::foreground::MUTED),
            palette: p,
        }
    }

    /// The subtle crimson glow on primary actions (BRIEF "Atmosphere").
    pub fn glow(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: alpha(self.palette.accent, akatsuki_tokens::effect::GLOW_OPACITY * opacity::accent::GLOW),
            offset: point(px(0.), px(2.)),
            blur_radius: px(akatsuki_tokens::effect::GLOW_BLUR / 2.),
            spread_radius: px(0.),
        }]
    }

    /// Elevation for floating surfaces (palette, pickers, toasts), tinted with the foreground.
    pub fn popover(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: alpha(self.palette.foreground, opacity::accent::WASH),
            offset: point(px(0.), px(12.)),
            blur_radius: px(akatsuki_tokens::space::S4),
            spread_radius: px(-akatsuki_tokens::space::S0_5),
        }]
    }

    /// A blurred crimson orb (BRIEF "Atmosphere"): a transparent dot whose shadow is the orb.
    pub fn orb(&self, size: f32) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: alpha(self.palette.accent, akatsuki_tokens::motion::orb::OPACITY),
            offset: point(px(0.), px(0.)),
            blur_radius: px(akatsuki_tokens::motion::orb::BLUR),
            spread_radius: px(size / 2.),
        }]
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
    /// Uppercase tracked labels.
    pub const LABEL: TextStyle = text::app::LABEL;
    /// Opened-thread subject (serif).
    pub const SUBJECT: TextStyle = text::app::SUBJECT;
    /// Section headings (serif).
    pub const HEADING: TextStyle = text::app::HEADING;
    /// Empty states (serif).
    pub const EMPTY: TextStyle = text::app::EMPTY;
    /// Brand display (serif).
    pub const DISPLAY: TextStyle = text::brand::DISPLAY;
}

/// Uppercases a label and spaces it out: GPUI 0.2.2 has no letter-spacing, so the tracking of
/// `text::app::LABEL` (0.12 em) is approximated with hair spaces between letters.
pub fn tracked(label: &str) -> String {
    let upper = label.to_uppercase();
    let mut out = String::with_capacity(upper.len() * 4);
    for (i, ch) in upper.chars().enumerate() {
        if i > 0 && ch != ' ' {
            out.push('\u{200A}');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_follow_the_tokens() {
        let light = Theme::new(Appearance::Light);
        assert_eq!(light.accent, solid(akatsuki_tokens::LIGHT.accent));
        assert_eq!(light.bg, solid(akatsuki_tokens::LIGHT.background));
        let dark = Theme::new(Appearance::Dark);
        assert_eq!(dark.accent, solid(akatsuki_tokens::DARK.accent));
        assert_eq!(dark.bg, solid(akatsuki_tokens::DARK.background));
        assert_eq!(dark.on_accent, solid(akatsuki_tokens::DARK.foreground));
        // Accent text is the token's lifted crimson, never the deep fill colour (D-399).
        assert_eq!(dark.accent_text, solid(akatsuki_tokens::DARK.accent_text));
        assert_ne!(dark.accent_text, dark.accent);
        assert_eq!(dark.link, dark.accent_text);
        assert_eq!(light.link, solid(akatsuki_tokens::LIGHT.accent));
    }

    #[test]
    fn theme_pref_resolves() {
        assert_eq!(ThemePref::System.resolve(WindowAppearance::Dark), Appearance::Dark);
        assert_eq!(ThemePref::Light.resolve(WindowAppearance::Dark), Appearance::Light);
        assert_eq!(ThemePref::Dark.resolve(WindowAppearance::Light), Appearance::Dark);
    }

    #[test]
    fn labels_are_uppercase_and_tracked() {
        assert_eq!(tracked("in"), "I\u{200A}N");
        assert_eq!(tracked("a b"), "A \u{200A}B");
    }
}
