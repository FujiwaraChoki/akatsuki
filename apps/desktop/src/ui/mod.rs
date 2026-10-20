//! GPUI views. The workspace owns the state; the other modules draw parts of it.

pub mod body;
pub mod input;
pub mod list;
pub mod overlays;
pub mod reading;
pub mod settings;
pub mod sidebar;
pub mod signin;
pub mod widgets;
pub mod workspace;

use gpui::{App, Global};

use crate::theme::Theme;

/// The theme in use, as a GPUI global so leaf elements (text fields) can read it.
#[derive(Clone, Copy, Debug)]
pub struct ThemeGlobal(pub Theme);

impl Global for ThemeGlobal {}

/// The current theme.
pub fn theme(cx: &App) -> Theme {
    cx.try_global::<ThemeGlobal>().map(|t| t.0).unwrap_or_else(|| Theme::new(akatsuki_tokens::Appearance::Light))
}
