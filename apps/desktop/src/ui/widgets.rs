//! Small building blocks styled from the theme ("First Light", the design spec §2): flat pills,
//! hairlines, keycaps and ink. Red appears only where the spec's red inventory allows it.

use gpui::{
    AnyElement, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Svg, div, linear_color_stop, linear_gradient,
    prelude::*, px, svg, transparent_black,
};

use crate::assets::icon_path;
use crate::theme::{Theme, fonts, styles};

/// An icon in a colour (16 px is the default size, §2.11).
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg().path(icon_path(name)).size(px(size)).flex_none().text_color(color)
}

/// One keycap: Geist Mono 11, `surface` fill, a `lineStrong` border with a 2 px bottom edge.
pub fn kbd(t: &Theme, keys: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(px(18.))
        .h(px(18.))
        .px(px(4.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .border_1()
        .border_b_2()
        .border_color(t.line_strong)
        .bg(t.surface)
        .font_family(fonts::MONO)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(styles::CAPTION.size))
        .line_height(px(12.))
        .text_color(t.secondary)
        .child(keys.into())
}

/// Keycaps side by side (`[G][I]`, `[Ctrl][K]`), no "then".
pub fn keys(t: &Theme, caps: &[String]) -> Div {
    div().flex().flex_none().items_center().gap(px(3.)).children(caps.iter().map(|c| kbd(t, c.clone())))
}

/// Keys as plain mono text (`G I` in the sidebar, as on the landing page).
pub fn key_text(t: &Theme, caps: &[String]) -> Div {
    div()
        .flex_none()
        .font_family(fonts::MONO)
        .text_size(px(styles::CAPTION.size))
        .text_color(t.muted)
        .child(caps.join(" "))
}

/// A section header: 12/500, sentence case, `tertiary` (replaces the old caps labels).
pub fn label(t: &Theme, text: &str) -> Div {
    div()
        .font_family(fonts::SANS)
        .text_size(px(styles::SECTION.size))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tertiary)
        .child(text.to_owned())
}

/// Mono label, sentence case as given (reader crumb, AI provenance line).
pub fn crumb(t: &Theme, text: &str) -> Div {
    div()
        .font_family(fonts::MONO)
        .text_size(px(styles::LABEL.size))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted)
        .child(text.to_owned())
}

/// Sentence-case display name for a triage category (web `TRIAGE_LABEL`).
pub fn triage_label(category: &str) -> String {
    match category {
        "important" => "Important".into(),
        "personal" => "Personal".into(),
        "updates" => "Updates".into(),
        "newsletters" => "Newsletter".into(),
        "promotions" => "Promotion".into(),
        "receipts" => "Receipt".into(),
        "calendar" => "Calendar".into(),
        "notifications" => "Notification".into(),
        "other" => "Other".into(),
        c => {
            let mut chars = c.chars();
            chars.next().map(|f| f.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// A label or triage chip: mono 11, sentence case as given, 18 px, hairline, radius 4.
pub fn chip(t: &Theme, text: &str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(18.))
        .px(px(5.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .border_1()
        .border_color(t.line_strong)
        .font_family(fonts::MONO)
        .text_size(px(styles::CAPTION.size))
        .line_height(px(12.))
        .text_color(t.muted)
        .child(text.to_owned())
}

/// Button kinds (§5.3): flat pills, no shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Accent fill: sign-in and billing only.
    Brand,
    /// Ink fill (Send, Save).
    Ink,
    /// Hairline pill.
    Secondary,
}

/// A pill button of a kind.
pub fn pill(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, kind: Kind) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(14.))
        .rounded_full()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(styles::BODY.size))
        .cursor_pointer();
    let (stroke, line_strong) = (t.stroke, t.line_strong);
    match kind {
        Kind::Brand => base.bg(t.accent).text_color(t.on_accent).child(text.into()),
        Kind::Ink => base.bg(t.fg).text_color(t.bg).child(text.into()),
        Kind::Secondary => base
            .border_1()
            .border_color(line_strong)
            .text_color(t.fg)
            .hover(move |s| s.border_color(stroke))
            .child(text.into()),
    }
}

/// A text button: ink for the primary action, hairline otherwise.
pub fn button(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    pill(t, id, text, if primary { Kind::Ink } else { Kind::Secondary })
}

/// A quiet row (sidebar, settings nav): 28 px, radius md; active is paper with a hairline ring.
pub fn ghost(t: &Theme, id: impl Into<ElementId>, active: bool) -> Stateful<Div> {
    let hover = t.surface_strong;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(28.))
        .pl(px(12.))
        .pr(px(8.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .border_1()
        .cursor_pointer()
        .when(active, |d| d.bg(t.bg).border_color(t.line).text_color(t.fg))
        .when(!active, |d| d.border_color(transparent_black()).text_color(t.tertiary).hover(move |s| s.bg(hover)))
}

/// An on/off switch: 28×16, on is ink, off a strong hairline, with a paper knob.
pub fn switch(t: &Theme, id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .w(px(28.))
        .h(px(16.))
        .p(px(2.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.fg } else { t.line_strong })
        .when(on, |d| d.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(t.bg))
}

/// The track of a segmented control.
pub fn segments(t: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .h(px(28.))
        .p(px(2.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .border_1()
        .border_color(t.line_strong)
        .bg(t.surface)
}

/// One option of a segmented control: the selected one is paper with a hairline ring.
pub fn segment(t: &Theme, id: impl Into<ElementId>, text: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
    let hover = t.fg;
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(22.))
        .px(px(10.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .border_1()
        .text_size(px(styles::META.size))
        .cursor_pointer()
        .when(selected, |d| d.bg(t.bg).border_color(t.line).text_color(t.fg).font_weight(FontWeight::MEDIUM))
        .when(!selected, |d| {
            d.border_color(transparent_black()).text_color(t.tertiary).hover(move |s| s.text_color(hover))
        })
        .child(text.into())
}

/// A field frame around a text field: strong hairline at rest, ink when focused.
pub fn field_frame(t: &Theme, focused: bool) -> Div {
    let stroke = t.stroke;
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(34.))
        .px(px(10.))
        .rounded(px(akatsuki_tokens::radius::MD))
        .border_1()
        .border_color(if focused { t.fg } else { t.line_strong })
        .when(!focused, |d| d.hover(move |s| s.border_color(stroke)))
        .bg(t.bg)
        .text_color(t.fg)
        .text_size(px(styles::BODY.size))
        .line_height(px(20.))
}

/// A raised surface (level 1): paper, one strong hairline, the soft shadow, radius lg.
pub fn raised(t: &Theme) -> Div {
    div().bg(t.bg).border_1().border_color(t.line_strong).rounded(px(akatsuki_tokens::radius::LG)).shadow(t.popover())
}

/// The horizon (§2.10): a 1 px dawn line, transparent at both ends. GPUI gradients take two stops,
/// so it is two half-width lines; `width` is 0..1 of the full line (it is drawn once on sign-in).
pub fn horizon(t: &Theme, width: f32) -> AnyElement {
    let opacity = match t.appearance {
        akatsuki_tokens::Appearance::Light => 0.8,
        akatsuki_tokens::Appearance::Dark => 0.95,
    };
    let half = gpui::relative(width.clamp(0., 1.) / 2.);
    let line = |angle: f32, from: Hsla, to: Hsla| {
        div().w(half).h(px(1.)).bg(linear_gradient(angle, linear_color_stop(from, 0.), linear_color_stop(to, 1.)))
    };
    div()
        .w_full()
        .h(px(1.))
        .flex()
        .justify_center()
        .opacity(opacity)
        .child(line(90., t.dawn.opacity(0.), t.dawn))
        .child(line(90., t.dawn, t.dawn.opacity(0.)))
        .into_any_element()
}

/// The red dot: the brand mark, never an unread marker (§2.4).
pub fn dot(t: &Theme, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded_full().bg(t.dawn)
}

/// Unread count text: `99+` past 99.
pub fn count(n: usize) -> String {
    if n > 99 { "99+".into() } else { n.to_string() }
}

/// A count in mono 11 `muted`.
pub fn count_el(t: &Theme, n: usize) -> Div {
    div()
        .flex_none()
        .min_w(px(22.))
        .flex()
        .justify_end()
        .font_family(fonts::MONO)
        .text_size(px(styles::CAPTION.size))
        .text_color(t.muted)
        .when(n > 0, |d| d.child(count(n)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_cap_at_99_plus() {
        assert_eq!(super::count(5), "5");
        assert_eq!(super::count(100), "99+");
    }
}
