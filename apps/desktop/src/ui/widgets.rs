//! Small building blocks styled from the theme (after ref/Siberia's `ui.rs`).

use gpui::{Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Svg, div, prelude::*, px, svg};

use crate::assets::icon_path;
use crate::theme::{Theme, fonts, styles, tracked};

/// An icon in a colour.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(icon_path(name)).size(px(size)).flex_none().text_color(color)
}

/// A key hint in Geist Mono.
pub fn kbd(t: &Theme, keys: impl Into<SharedString>) -> Div {
    div()
        .font_family(fonts::MONO)
        .text_size(px(styles::CAPTION.size))
        .px(px(5.))
        .py(px(1.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .border_1()
        .border_color(t.border_subtle)
        .text_color(t.muted)
        .child(keys.into())
}

/// An uppercase, tracked label (BRIEF: "uppercase, wide tracking").
pub fn label(t: &Theme, text: &str) -> Div {
    div()
        .font_family(fonts::SANS)
        .text_size(px(styles::LABEL.size))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted)
        .child(tracked(text))
}

/// A text button. Primary buttons are crimson with a subtle glow.
pub fn button(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(32.))
        .px(px(14.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(styles::BODY.size))
        .cursor_pointer();
    if primary {
        let (bg, hover) = (t.accent, t.accent_core);
        base.bg(bg).text_color(t.on_accent).shadow(t.glow()).hover(move |s| s.bg(hover)).child(text.into())
    } else {
        let wash = t.accent_wash;
        base.border_1().border_color(t.border).text_color(t.fg).hover(move |s| s.bg(wash)).child(text.into())
    }
}

/// A quiet row button (sidebar entries, menu rows).
pub fn ghost(t: &Theme, id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    let wash = t.accent_wash;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(30.))
        .px(px(10.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .cursor_pointer()
        .when(active, |d| d.bg(t.accent_tint).text_color(t.fg))
        .when(!active, |d| d.text_color(t.secondary).hover(move |s| s.bg(wash)))
}

/// An on/off switch.
pub fn switch(t: &Theme, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .w(px(32.))
        .h(px(18.))
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.border })
        .when(on, |d| d.justify_end())
        .child(div().size(px(14.)).rounded_full().bg(t.bg))
}

/// One option of a segmented control.
pub fn segment(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
    let wash = t.accent_wash;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .text_size(px(styles::META.size))
        .cursor_pointer()
        .when(selected, |d| d.bg(t.accent_tint).text_color(t.fg).font_weight(FontWeight::MEDIUM))
        .when(!selected, |d| d.text_color(t.muted).hover(move |s| s.bg(wash)))
        .child(text.into())
}

/// A bordered field frame around a text field.
pub fn field_frame(t: &Theme, focused: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(34.))
        .px(px(10.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .border_1()
        .border_color(if focused { t.accent_text } else { t.border })
        .bg(t.bg)
        .text_color(t.fg)
        .text_size(px(styles::BODY.size))
        .line_height(px(20.))
}

/// Unread count text: `99+` past 99.
pub fn count(n: usize) -> String {
    if n > 99 { "99+".into() } else { n.to_string() }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_cap_at_99_plus() {
        assert_eq!(super::count(5), "5");
        assert_eq!(super::count(100), "99+");
    }
}
