//! Embedded assets, following Siberia's approach: fonts are compiled into the binary and handed
//! to GPUI's text system at start-up; other assets (icons) are served by [`Assets`].
//!
//! The font files are the ones `packages/tokens` ships (single source, BRIEF "Design theme"):
//! Instrument Serif for the brand, Geist and Geist Mono for dense UI. Licences: SIL OFL 1.1, see
//! `packages/tokens/fonts/OFL-*.txt`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! font {
    ($file:literal) => {
        ($file, include_bytes!(concat!("../../../packages/tokens/fonts/", $file)) as &[u8])
    };
}

/// `(file name, bytes)` of every bundled font.
pub const FONTS: &[(&str, &[u8])] = &[
    font!("InstrumentSerif-Regular.ttf"),
    font!("InstrumentSerif-Italic.ttf"),
    font!("Geist-Regular.ttf"),
    font!("Geist-Medium.ttf"),
    font!("Geist-SemiBold.ttf"),
    font!("Geist-Bold.ttf"),
    font!("Geist-Italic.ttf"),
    font!("Geist-MediumItalic.ttf"),
    font!("Geist-SemiBoldItalic.ttf"),
    font!("Geist-BoldItalic.ttf"),
    font!("GeistMono-Regular.ttf"),
    font!("GeistMono-Medium.ttf"),
    font!("GeistMono-Italic.ttf"),
];

macro_rules! icon {
    ($name:literal) => {
        (concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8])
    };
}

/// SVG icons (Siberia's set plus a few drawn for mail), by path. Use [`icon_path`].
const FILES: &[(&str, &[u8])] = &[
    icon!("all"),
    icon!("archive"),
    icon!("arrow-left"),
    icon!("bell"),
    icon!("bookmark"),
    icon!("check"),
    icon!("chevron-down"),
    icon!("chevron-up"),
    icon!("clock"),
    icon!("close"),
    icon!("compose"),
    icon!("download"),
    icon!("external"),
    icon!("file"),
    icon!("image"),
    icon!("inbox"),
    icon!("info"),
    icon!("link"),
    icon!("message"),
    icon!("more"),
    icon!("octagon"),
    icon!("paperclip"),
    icon!("plus"),
    icon!("search"),
    icon!("send"),
    icon!("settings"),
    icon!("sparkle"),
    icon!("star"),
    icon!("tag"),
    icon!("trash"),
    icon!("triangle-alert"),
    icon!("undo"),
    icon!("user"),
];

/// The asset path of an icon, e.g. `icons/inbox.svg`.
pub fn icon_path(name: &str) -> gpui::SharedString {
    format!("icons/{name}.svg").into()
}

/// The asset source passed to `Application::with_assets`.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(p, _)| *p == path).map(|(_, data)| Cow::Borrowed(*data)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_plain_svg() {
        assert!(FILES.len() > 20);
        for (path, bytes) in FILES {
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(text.starts_with("<svg"), "{path}");
            assert!(!text.contains("<script") && !text.contains("href"), "{path}");
        }
        assert!(Assets.load("icons/inbox.svg").unwrap().is_some());
        assert_eq!(icon_path("star"), "icons/star.svg");
    }

    #[test]
    fn bundled_fonts_are_the_token_fonts() {
        for (_, file) in akatsuki_tokens::font::serif::FILES {
            assert!(FONTS.iter().any(|(f, _)| f == file), "{file} is not bundled");
        }
        for (name, bytes) in FONTS {
            // TrueType files start with 0x00010000 or "true".
            assert!(bytes.starts_with(&[0, 1, 0, 0]) || bytes.starts_with(b"true"), "{name} is not a TTF");
        }
    }
}
