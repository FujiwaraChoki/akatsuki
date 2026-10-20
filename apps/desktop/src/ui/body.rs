//! Draws the native layout (`crate::render::body`) with GPUI: styled runs, clickable links,
//! quotes, lists, data tables, inline images and placeholders for blocked remote images. Only
//! images whose bytes are already in hand (inline `cid:` parts and small `data:` URLs) are drawn;
//! nothing here ever hands a URL to GPUI's image loader.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use gpui::{
    AnyElement, FontStyle, FontWeight, HighlightStyle, ImageFormat, InteractiveText, SharedString, StrikethroughStyle,
    StyledText, UnderlineStyle, div, img, prelude::*, px,
};

use akatsuki_mime::ImageSource;

use super::widgets::icon;
use crate::render::body::{Block, Inline};
use crate::theme::{Theme, fonts, styles};

/// Inline image bytes by part id.
pub type InlineImages = HashMap<String, (String, Vec<u8>)>;

fn format_of(mime: &str) -> Option<ImageFormat> {
    Some(match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        _ => return None,
    })
}

fn data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, b64) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    Some((mime.to_ascii_lowercase(), STANDARD.decode(b64).ok()?))
}

fn inline_el(t: &Theme, inline: &Inline, id: SharedString) -> AnyElement {
    let mut highlights = Vec::new();
    for s in &inline.spans {
        let mut h = HighlightStyle::default();
        if s.style.bold {
            h.font_weight = Some(FontWeight::SEMIBOLD);
        }
        if s.style.italic {
            h.font_style = Some(FontStyle::Italic);
        }
        if s.style.underline || s.link.is_some() {
            h.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(t.fg), wavy: false });
        }
        if s.style.strike {
            h.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(t.muted) });
        }
        if s.style.code {
            h.background_color = Some(t.surface_strong);
        }
        if s.link.is_some() {
            h.color = Some(t.fg);
        }
        highlights.push((s.range.clone(), h));
    }
    let links = inline.links();
    let text = StyledText::new(inline.text.clone()).with_highlights(highlights);
    if links.is_empty() {
        return div().child(text).into_any_element();
    }
    let ranges = links.iter().map(|(r, _)| r.clone()).collect();
    let targets: Vec<String> = links.into_iter().map(|(_, l)| l).collect();
    InteractiveText::new(id, text)
        .on_click(ranges, move |ix, _window, cx| {
            if let Some(url) = targets.get(ix) {
                // The sanitizer only keeps http, https, mailto and tel targets.
                cx.open_url(url);
            }
        })
        .into_any_element()
}

fn image_el(t: &Theme, alt: &str, source: &ImageSource, images: &InlineImages) -> AnyElement {
    let bytes = match source {
        ImageSource::Cid { part_id } => images.get(part_id).cloned(),
        ImageSource::Data { url } => data_url(url),
        ImageSource::Remote { .. } => None,
    };
    if let Some((mime, bytes)) = bytes
        && let Some(format) = format_of(&mime)
    {
        let image = Arc::new(gpui::Image::from_bytes(format, bytes));
        return div().max_w(px(640.)).child(img(image).max_w(px(640.)).max_h(px(480.))).into_any_element();
    }
    let what = if alt.trim().is_empty() { "Image".to_owned() } else { alt.trim().to_owned() };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(akatsuki_tokens::radius::SM))
        .border_1()
        .border_color(t.line)
        .text_size(px(styles::META.size))
        .text_color(t.muted)
        .child(icon("image", 13., t.muted))
        .child(what)
        .into_any_element()
}

/// Draws blocks. `id` prefixes element ids (they must be unique within the view).
pub fn blocks(t: &Theme, blocks: &[Block], id: &str, images: &InlineImages) -> Vec<AnyElement> {
    blocks.iter().enumerate().map(|(i, b)| block(t, b, &format!("{id}-{i}"), images)).collect()
}

fn block(t: &Theme, b: &Block, id: &str, images: &InlineImages) -> AnyElement {
    match b {
        Block::Para(inline) => inline_el(t, inline, id.to_owned().into()),
        Block::Heading(level, inline) => {
            let size = match level {
                1 => 22.,
                2 => 19.,
                3 => 17.,
                _ => styles::READING.size,
            };
            div()
                .text_size(px(size))
                .line_height(px(size * 1.35))
                .font_weight(FontWeight::SEMIBOLD)
                .child(inline_el(t, inline, id.to_owned().into()))
                .into_any_element()
        }
        Block::Quote(inner) => div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pl(px(12.))
            .border_l_2()
            .border_color(t.line_strong)
            .text_color(t.secondary)
            .children(blocks(t, inner, id, images))
            .into_any_element(),
        Block::List { ordered, items } => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(items.iter().enumerate().map(|(n, item)| {
                let marker = if *ordered { format!("{}.", n + 1) } else { "•".to_owned() };
                div().flex().gap(px(8.)).child(div().w(px(18.)).flex_none().text_color(t.muted).child(marker)).child(
                    div().flex_1().min_w(px(0.)).flex().flex_col().gap(px(4.)).children(blocks(
                        t,
                        item,
                        &format!("{id}-{n}"),
                        images,
                    )),
                )
            }))
            .into_any_element(),
        Block::Pre(text) => div()
            .font_family(fonts::MONO)
            .text_size(px(styles::META.size))
            .p(px(10.))
            .rounded(px(akatsuki_tokens::radius::MD))
            .bg(t.surface)
            .child(text.clone())
            .into_any_element(),
        Block::Rule => div().h(px(1.)).w_full().bg(t.line).into_any_element(),
        Block::Image { alt, source } => image_el(t, alt, source, images),
        Block::Table(rows) => div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(t.line)
            .rounded(px(akatsuki_tokens::radius::SM))
            .children(rows.iter().enumerate().map(|(r, cells)| {
                div().flex().when(r > 0, |d| d.border_t_1().border_color(t.line)).children(
                    cells.iter().enumerate().map(|(c, cell)| {
                        div().flex_1().min_w(px(0.)).p(px(6.)).flex().flex_col().gap(px(4.)).children(blocks(
                            t,
                            cell,
                            &format!("{id}-{r}-{c}"),
                            images,
                        ))
                    }),
                )
            }))
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode_and_unknown_types_are_refused() {
        let (mime, bytes) = data_url("data:image/PNG;base64,AAEC").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, vec![0, 1, 2]);
        assert!(data_url("data:image/png,raw").is_none());
        assert!(format_of("image/svg+xml").is_none(), "SVG is never drawn from mail");
        assert!(format_of("image/gif").is_some());
    }
}
