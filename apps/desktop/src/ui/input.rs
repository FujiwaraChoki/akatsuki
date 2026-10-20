//! Text fields (adapted from ref/Siberia's `input.rs`, the owner's own GPUI code): single-line
//! fields for search, the palette, sign-in and settings, a masked one for keys and codes, and a
//! wrapping multi-line one for the compose body. They implement GPUI's input handler, so IME,
//! dead keys and the platform's clipboard work.

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ScrollWheelEvent, SharedString, Size, Style, TextAlign,
    TextRun, UTF16Selection, Window, WrappedLine, actions, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use super::theme;

actions!(
    field,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        Up,
        Down,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Submit,
        SubmitBack,
        Cancel,
        Newline
    ]
);

/// What a field reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldEvent {
    /// The text changed.
    Changed,
    /// Enter.
    Submit,
    /// Shift+Enter in a single-line field.
    SubmitBack,
    /// Escape.
    Cancel,
}

/// A text field.
pub struct TextField {
    focus_handle: FocusHandle,
    /// The text.
    pub content: String,
    placeholder: SharedString,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    multiline: bool,
    max_lines: usize,
    masked: bool,
    layout: Option<FieldLayout>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_row: usize,
    goal_x: Option<Pixels>,
    selecting: bool,
}

#[derive(Clone)]
struct Row {
    range: Range<usize>,
    line: usize,
    x_offset: Pixels,
    last_in_line: bool,
}

#[derive(Clone)]
struct FieldLayout {
    lines: Vec<(usize, WrappedLine)>,
    rows: Vec<Row>,
    line_height: Pixels,
    mask: Option<Vec<usize>>,
}

const MASK: &str = "•";

impl FieldLayout {
    fn new(lines: Vec<(usize, WrappedLine)>, line_height: Pixels) -> Self {
        let mut rows = Vec::new();
        for (i, (start, line)) in lines.iter().enumerate() {
            let layout = &line.unwrapped_layout;
            let mut starts = vec![0];
            for b in &line.wrap_boundaries {
                if let Some(glyph) = layout.runs.get(b.run_ix).and_then(|r| r.glyphs.get(b.glyph_ix)) {
                    starts.push(glyph.index);
                }
            }
            for (j, &s) in starts.iter().enumerate() {
                let e = starts.get(j + 1).copied().unwrap_or(line.len());
                rows.push(Row {
                    range: start + s..start + e,
                    line: i,
                    x_offset: layout.x_for_index(s),
                    last_in_line: j + 1 == starts.len(),
                });
            }
        }
        Self { lines, rows, line_height, mask: None }
    }

    fn masking(mut self, content: &str) -> Self {
        let bounds: Vec<usize> = content.char_indices().map(|(i, _)| i).chain([content.len()]).collect();
        self.mask = Some(bounds);
        for i in 0..self.rows.len() {
            let range = self.rows[i].range.clone();
            self.rows[i].range = self.content_offset(range.start)..self.content_offset(range.end);
        }
        self
    }

    fn to_display(&self, offset: usize) -> usize {
        match &self.mask {
            Some(bounds) => bounds.partition_point(|&b| b < offset) * MASK.len(),
            None => offset,
        }
    }

    fn content_offset(&self, offset: usize) -> usize {
        match &self.mask {
            Some(bounds) => bounds[(offset / MASK.len()).min(bounds.len() - 1)],
            None => offset,
        }
    }

    fn row_for(&self, offset: usize) -> usize {
        self.rows
            .iter()
            .position(|r| {
                offset >= r.range.start && (offset < r.range.end || (offset == r.range.end && r.last_in_line))
            })
            .unwrap_or(self.rows.len().saturating_sub(1))
    }

    fn position(&self, offset: usize) -> Point<Pixels> {
        let r = self.row_for(offset);
        let Some(row) = self.rows.get(r) else { return point(px(0.), px(0.)) };
        point(self.x_in_row(offset, row), self.line_height * r as f32)
    }

    fn x_in_row(&self, offset: usize, row: &Row) -> Pixels {
        let (start, line) = &self.lines[row.line];
        line.unwrapped_layout.x_for_index(self.to_display(offset).saturating_sub(*start)) - row.x_offset
    }

    fn index_in_row(&self, r: usize, x: Pixels) -> usize {
        let Some(row) = self.rows.get(r) else { return 0 };
        let (start, line) = &self.lines[row.line];
        let local = self.content_offset(line.unwrapped_layout.closest_index_for_x(x + row.x_offset) + start);
        local.clamp(row.range.start, row.range.end)
    }
}

impl EventEmitter<FieldEvent> for TextField {}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextField {
    /// A single-line field.
    pub fn new(placeholder: &str, cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            placeholder: SharedString::from(placeholder.to_string()),
            selected: 0..0,
            reversed: false,
            marked: None,
            multiline: false,
            max_lines: 1,
            masked: false,
            layout: None,
            last_bounds: None,
            scroll_row: 0,
            goal_x: None,
            selecting: false,
        }
    }

    /// A wrapping field that grows to `max_lines` rows before scrolling.
    pub fn multiline(mut self, max_lines: usize) -> Self {
        self.multiline = true;
        self.max_lines = max_lines.max(1);
        self
    }

    /// Dots instead of characters; the text cannot be copied out.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self.multiline = false;
        self
    }

    /// Replaces the text and puts the caret at the end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        self.content = text.to_string();
        self.selected = self.content.len()..self.content.len();
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        self.scroll_row = 0;
        cx.notify();
    }

    /// Moves the caret to the start (replies start above the quote).
    pub fn caret_to_start(&mut self, cx: &mut Context<'_, Self>) {
        self.move_to(0, cx);
    }

    /// Changes the placeholder (the palette's field serves every picker).
    pub fn set_placeholder(&mut self, placeholder: &str, cx: &mut Context<'_, Self>) {
        self.placeholder = SharedString::from(placeholder.to_owned());
        cx.notify();
    }

    /// The selected byte range (empty at the caret).
    pub fn selection(&self) -> Range<usize> {
        self.selected.clone()
    }

    /// The caret's byte offset.
    pub fn caret(&self) -> usize {
        self.cursor()
    }

    /// Replaces the selection with `text` (snippets, dictation) and puts the caret after it.
    pub fn insert(&mut self, text: &str, cx: &mut Context<'_, Self>) {
        self.replace(self.selected.clone(), text, cx);
    }

    /// Replaces `range` with `text` (a `;shortcut` expanding into its snippet).
    pub fn replace_range(&mut self, range: Range<usize>, text: &str, cx: &mut Context<'_, Self>) {
        self.replace(range, text, cx);
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<'_, Self>) {
        self.selected = offset..offset;
        self.reversed = false;
        self.goal_x = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<'_, Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        cx.notify();
    }

    fn prev(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    fn next(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.content.len())
    }

    fn prev_word(&self, offset: usize) -> usize {
        if self.masked {
            return 0;
        }
        self.content
            .split_word_bound_indices()
            .rev()
            .find(|(i, w)| *i < offset && w.chars().any(char::is_alphanumeric))
            .map_or(0, |(i, _)| i)
    }

    fn next_word(&self, offset: usize) -> usize {
        if self.masked {
            return self.content.len();
        }
        self.content
            .split_word_bound_indices()
            .find(|(i, w)| i + w.len() > offset && w.chars().any(char::is_alphanumeric))
            .map_or(self.content.len(), |(i, w)| i + w.len())
    }

    fn vertical(&mut self, delta: isize) -> Option<usize> {
        let layout = self.layout.as_ref()?;
        let cursor = self.cursor();
        let row = layout.row_for(cursor);
        let x = *self.goal_x.get_or_insert(layout.position(cursor).x);
        let target = row as isize + delta;
        Some(if target < 0 {
            0
        } else if target as usize >= layout.rows.len() {
            self.content.len()
        } else {
            layout.index_in_row(target as usize, x)
        })
    }

    fn row_bounds(&self) -> Range<usize> {
        match &self.layout {
            Some(layout) if self.multiline => {
                layout.rows.get(layout.row_for(self.cursor())).map_or(0..self.content.len(), |r| r.range.clone())
            }
            _ => 0..self.content.len(),
        }
    }

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<'_, Self>) -> Range<usize> {
        let text = if self.multiline {
            text.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            text.replace(['\n', '\r'], " ")
        };
        let range = self.clamp(range.start)..self.clamp(range.end);
        let range = range.start.min(range.end)..range.end;
        self.content.replace_range(range.clone(), &text);
        let caret = range.start + text.len();
        self.selected = caret..caret;
        self.reversed = false;
        self.marked = None;
        self.goal_x = None;
        cx.emit(FieldEvent::Changed);
        cx.notify();
        range.start..caret
    }

    fn paste(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace(self.selected.clone(), &text, cx);
        }
    }

    fn index_for(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(layout)) = (self.last_bounds.as_ref(), self.layout.as_ref()) else { return 0 };
        let local = position - bounds.origin;
        let row = ((local.y / layout.line_height).floor().max(0.) as usize + self.scroll_row)
            .min(layout.rows.len().saturating_sub(1));
        layout.index_in_row(row, local.x)
    }

    fn utf16_to_utf8(&self, offset: usize) -> usize {
        let mut units = 0;
        for (i, ch) in self.content.char_indices() {
            if units >= offset {
                return i;
            }
            units += ch.len_utf16();
        }
        self.content.len()
    }

    fn utf8_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset.min(self.content.len())].encode_utf16().count()
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<String> {
        let r = self.utf16_to_utf8(range.start)..self.utf16_to_utf8(range.end);
        actual.replace(self.utf8_to_utf16(r.start)..self.utf8_to_utf16(r.end));
        Some(self.content[r].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<'_, Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.utf8_to_utf16(self.selected.start)..self.utf8_to_utf16(self.selected.end),
            reversed: self.reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<'_, Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|r| self.utf8_to_utf16(r.start)..self.utf8_to_utf16(r.end))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<'_, Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_utf8(r.start)..self.utf16_to_utf8(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        self.replace(range, text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let range = range
            .map(|r| self.utf16_to_utf8(r.start)..self.utf16_to_utf8(r.end))
            .or(self.marked.clone())
            .unwrap_or(self.selected.clone());
        let inserted = self.replace(range, text, cx);
        self.marked = (!inserted.is_empty()).then_some(inserted);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let scroll = layout.line_height * self.scroll_row as f32;
        let start = layout.position(self.utf16_to_utf8(range.start));
        let end = layout.position(self.utf16_to_utf8(range.end));
        let top = bounds.top() + start.y - scroll;
        Some(Bounds::from_corners(
            point(bounds.left() + start.x, top),
            point(bounds.left() + end.x.max(start.x), top + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        p: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<'_, Self>,
    ) -> Option<usize> {
        Some(self.utf8_to_utf16(self.index_for(p)))
    }
}

struct FieldElement {
    field: Entity<TextField>,
}

struct FieldPrepaint {
    layout: FieldLayout,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
}

impl IntoElement for FieldElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

fn display_text(field: &TextField, window: &Window, cx: &App) -> (SharedString, TextRun) {
    let style = window.text_style();
    let (text, color) = if field.content.is_empty() {
        (field.placeholder.clone(), theme(cx).muted)
    } else if field.masked {
        (SharedString::from(MASK.repeat(field.content.chars().count())), style.color)
    } else {
        (SharedString::from(field.content.clone()), style.color)
    };
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    (text, run)
}

fn shape(
    text: &SharedString,
    run: &TextRun,
    font_size: Pixels,
    width: Option<Pixels>,
    window: &Window,
) -> Vec<(usize, WrappedLine)> {
    let lines = window
        .text_system()
        .shape_text(text.clone(), font_size, std::slice::from_ref(run), width, None)
        .unwrap_or_default();
    let mut start = 0;
    lines
        .into_iter()
        .map(|line| {
            let entry = (start, line);
            start += entry.1.len() + 1;
            entry
        })
        .collect()
}

impl Element for FieldElement {
    type RequestLayoutState = ();
    type PrepaintState = FieldPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let field = self.field.read(cx);
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !field.multiline {
            style.size.height = line_height.into();
            return (window.request_layout(style, [], cx), ());
        }
        let (text, run) = display_text(field, window, cx);
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let max_lines = field.max_lines;
        let id = window.request_measured_layout(
            style,
            move |known: Size<Option<Pixels>>, available: Size<AvailableSpace>, window, _| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(w) => Some(w),
                    _ => None,
                });
                let rows: usize =
                    shape(&text, &run, font_size, width, window).iter().map(|(_, l)| l.wrap_boundaries.len() + 1).sum();
                size(width.unwrap_or_default(), line_height * rows.clamp(1, max_lines) as f32)
            },
        );
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> FieldPrepaint {
        let t = theme(cx);
        let field = self.field.read(cx);
        let (text, run) = display_text(field, window, cx);
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let width = field.multiline.then_some(bounds.size.width);
        let mut layout = FieldLayout::new(shape(&text, &run, font_size, width, window), line_height);
        if field.masked && !field.content.is_empty() {
            layout = layout.masking(&field.content);
        }
        if field.content.is_empty() {
            layout.rows.truncate(1);
        }
        let visible = ((bounds.size.height / line_height).round() as usize).max(1);
        let caret_row = if field.content.is_empty() { 0 } else { layout.row_for(field.cursor()) };
        let mut scroll = field.scroll_row.min(layout.rows.len().saturating_sub(visible));
        if caret_row < scroll {
            scroll = caret_row;
        } else if caret_row >= scroll + visible {
            scroll = caret_row + 1 - visible;
        }
        let shift = line_height * scroll as f32;
        let (selection, cursor) = if field.content.is_empty() || field.selected.is_empty() {
            let at = if field.content.is_empty() { point(px(0.), px(0.)) } else { layout.position(field.cursor()) };
            let quad = fill(
                Bounds::new(point(bounds.left() + at.x, bounds.top() + at.y - shift), size(px(1.5), line_height)),
                t.dawn,
            );
            (Vec::new(), Some(quad))
        } else {
            let sel = field.selected.clone();
            let quads = layout
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.range.start <= sel.end && row.range.end >= sel.start)
                .filter_map(|(i, row)| {
                    let (s, e) = (sel.start.max(row.range.start), sel.end.min(row.range.end));
                    let past_end = sel.end > row.range.end && row.last_in_line;
                    if s >= e && !past_end {
                        return None;
                    }
                    let x0 = layout.x_in_row(s, row);
                    let x1 = layout.x_in_row(e, row) + if past_end { px(5.) } else { px(0.) };
                    let top = bounds.top() + line_height * i as f32 - shift;
                    Some(fill(
                        Bounds::from_corners(
                            point(bounds.left() + x0, top),
                            point(bounds.left() + x1, top + line_height),
                        ),
                        t.dawn.opacity(0.22),
                    ))
                })
                .collect();
            (quads, None)
        };
        self.field.update(cx, |f, _| f.scroll_row = scroll);
        FieldPrepaint { layout, cursor, selection }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut FieldPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus, scroll) = {
            let field = self.field.read(cx);
            (field.focus_handle.clone(), field.scroll_row)
        };
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.field.clone()), cx);
        let layout = &prepaint.layout;
        let line_height = layout.line_height;
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for quad in prepaint.selection.drain(..) {
                window.paint_quad(quad);
            }
            let mut row = 0;
            for (_, line) in &layout.lines {
                let y = line_height * (row as f32 - scroll as f32);
                line.paint(point(bounds.left(), bounds.top() + y), line_height, TextAlign::Left, None, window, cx).ok();
                row += line.wrap_boundaries.len() + 1;
            }
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        let layout = prepaint.layout.clone();
        self.field.update(cx, |field, _| {
            field.layout = Some(layout);
            field.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .key_context(if self.multiline { "TextField multiline" } else { "TextField" })
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(|f, _: &Backspace, _, cx| {
                if f.selected.is_empty() {
                    let prev = f.prev(f.cursor());
                    f.select_to(prev, cx);
                }
                f.replace(f.selected.clone(), "", cx);
            }))
            .on_action(cx.listener(|f, _: &DeleteWordLeft, _, cx| {
                if f.selected.is_empty() {
                    let prev = f.prev_word(f.cursor());
                    f.select_to(prev, cx);
                }
                f.replace(f.selected.clone(), "", cx);
            }))
            .on_action(cx.listener(|f, _: &Delete, _, cx| {
                if f.selected.is_empty() {
                    let next = f.next(f.cursor());
                    f.select_to(next, cx);
                }
                f.replace(f.selected.clone(), "", cx);
            }))
            .on_action(cx.listener(|f, _: &Left, _, cx| {
                let to = if f.selected.is_empty() { f.prev(f.cursor()) } else { f.selected.start };
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &Right, _, cx| {
                let to = if f.selected.is_empty() { f.next(f.cursor()) } else { f.selected.end };
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &WordLeft, _, cx| {
                let to = f.prev_word(f.cursor());
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &WordRight, _, cx| {
                let to = f.next_word(f.cursor());
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &Up, _, cx| {
                if let Some(to) = f.vertical(-1) {
                    let goal = f.goal_x;
                    f.move_to(to, cx);
                    f.goal_x = goal;
                }
            }))
            .on_action(cx.listener(|f, _: &Down, _, cx| {
                if let Some(to) = f.vertical(1) {
                    let goal = f.goal_x;
                    f.move_to(to, cx);
                    f.goal_x = goal;
                }
            }))
            .on_action(cx.listener(|f, _: &SelectUp, _, cx| {
                if let Some(to) = f.vertical(-1) {
                    f.select_to(to, cx);
                }
            }))
            .on_action(cx.listener(|f, _: &SelectDown, _, cx| {
                if let Some(to) = f.vertical(1) {
                    f.select_to(to, cx);
                }
            }))
            .on_action(cx.listener(|f, _: &SelectLeft, _, cx| {
                let to = f.prev(f.cursor());
                f.select_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &SelectRight, _, cx| {
                let to = f.next(f.cursor());
                f.select_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &SelectAll, _, cx| {
                f.selected = 0..f.content.len();
                cx.notify();
            }))
            .on_action(cx.listener(|f, _: &Home, _, cx| {
                let to = f.row_bounds().start;
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &End, _, cx| {
                let to = f.row_bounds().end;
                f.move_to(to, cx);
            }))
            .on_action(cx.listener(|f, _: &Copy, _, cx| {
                if !f.selected.is_empty() && !f.masked {
                    cx.write_to_clipboard(ClipboardItem::new_string(f.content[f.selected.clone()].to_string()));
                }
            }))
            .on_action(cx.listener(|f, _: &Cut, _, cx| {
                if !f.selected.is_empty() && !f.masked {
                    cx.write_to_clipboard(ClipboardItem::new_string(f.content[f.selected.clone()].to_string()));
                    f.replace(f.selected.clone(), "", cx);
                }
            }))
            .on_action(cx.listener(|f, _: &Paste, _, cx| f.paste(cx)))
            .on_action(cx.listener(|f, _: &Newline, _, cx| {
                if f.multiline {
                    f.replace(f.selected.clone(), "\n", cx);
                }
            }))
            .on_action(cx.listener(|f, _: &Submit, _, cx| {
                if f.multiline {
                    f.replace(f.selected.clone(), "\n", cx);
                } else {
                    cx.emit(FieldEvent::Submit);
                }
            }))
            .on_action(cx.listener(|_, _: &SubmitBack, _, cx| cx.emit(FieldEvent::SubmitBack)))
            .on_action(cx.listener(|_, _: &Cancel, _, cx| cx.emit(FieldEvent::Cancel)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|f, e: &MouseDownEvent, window, cx| {
                    window.focus(&f.focus_handle);
                    f.selecting = true;
                    let at = f.index_for(e.position);
                    if e.modifiers.shift { f.select_to(at, cx) } else { f.move_to(at, cx) }
                }),
            )
            .on_mouse_move(cx.listener(|f, e: &MouseMoveEvent, _, cx| {
                if f.selecting {
                    let at = f.index_for(e.position);
                    f.select_to(at, cx);
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|f, _: &MouseUpEvent, _, _| f.selecting = false))
            .on_scroll_wheel(cx.listener(|f, e: &ScrollWheelEvent, window, cx| {
                let Some(layout) = f.layout.as_ref() else { return };
                let rows = layout.rows.len();
                if rows <= f.max_lines {
                    return;
                }
                let dy = e.delta.pixel_delta(window.line_height()).y;
                let step = (-dy / layout.line_height).round() as isize;
                f.scroll_row = (f.scroll_row as isize + step).clamp(0, (rows - f.max_lines) as isize) as usize;
                cx.notify();
                cx.stop_propagation();
            }))
            .child(FieldElement { field: cx.entity() })
    }
}

/// Text field key bindings (GPUI actions, scoped to the field's key context). App shortcuts go
/// through `crate::keys` instead.
pub fn bind_keys(cx: &mut App) {
    use gpui::KeyBinding;
    let ctx = Some("TextField");
    let multi = Some("TextField && multiline");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new("shift-enter", SubmitBack, ctx),
        KeyBinding::new("escape", Cancel, ctx),
        KeyBinding::new("up", Up, multi),
        KeyBinding::new("down", Down, multi),
        KeyBinding::new("shift-up", SelectUp, multi),
        KeyBinding::new("shift-down", SelectDown, multi),
        KeyBinding::new("shift-enter", Newline, multi),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("alt-left", WordLeft, ctx),
            KeyBinding::new("alt-right", WordRight, ctx),
            KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("cmd-left", Home, ctx),
            KeyBinding::new("cmd-right", End, ctx),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-left", WordLeft, ctx),
            KeyBinding::new("ctrl-right", WordRight, ctx),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masked_offsets_map_to_dots() {
        let layout =
            FieldLayout { lines: Vec::new(), rows: Vec::new(), line_height: px(16.), mask: None }.masking("aé🙂b");
        assert_eq!([0, 1, 3, 7, 8].map(|o| layout.to_display(o)), [0, 3, 6, 9, 12]);
        assert_eq!([0, 3, 6, 9, 12, 99].map(|d| layout.content_offset(d)), [0, 1, 3, 7, 8, 8]);
    }
}
