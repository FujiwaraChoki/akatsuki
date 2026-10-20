//! The native renderer's layout model (ARCHITECTURE §14.3, BRIEF decision 4): the sanitizer's safe
//! tree (`crates/mime`) becomes a short list of blocks with styled inline runs, which the reading
//! pane draws with GPUI. Nothing here can run code or load a URL: links are data, remote images
//! are placeholders until the user opens the original.
//!
//! Colours and fonts from the email are not used (the theme's colours keep every message readable
//! in light and dark); weight, slant, underline, strike-through and monospace are.

use std::ops::Range;

use akatsuki_mime::{Display, ImageSource, SafeElement, SafeNode};

/// Inline style flags of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
    /// Underline.
    pub underline: bool,
    /// Strike-through.
    pub strike: bool,
    /// Monospace.
    pub code: bool,
}

/// A styled byte range of an [`Inline`]'s text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte range.
    pub range: Range<usize>,
    /// Style.
    pub style: Style,
    /// Link target (`http`, `https`, `mailto`, `tel`).
    pub link: Option<String>,
}

/// A paragraph of text with styled runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    /// Text.
    pub text: String,
    /// Styled runs, in order, not overlapping.
    pub spans: Vec<Span>,
}

impl Inline {
    fn push(&mut self, text: &str, style: Style, link: Option<&str>) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        if style != Style::default() || link.is_some() {
            if let Some(last) = self.spans.last_mut()
                && last.range.end == start
                && last.style == style
                && last.link.as_deref() == link
            {
                last.range.end = self.text.len();
                return;
            }
            self.spans.push(Span { range: start..self.text.len(), style, link: link.map(str::to_owned) });
        }
    }

    /// Trims surrounding whitespace, keeping the spans consistent.
    fn trimmed(mut self) -> Self {
        let lead = self.text.len() - self.text.trim_start().len();
        let keep = self.text.trim().len();
        self.text = self.text[lead..lead + keep].to_owned();
        self.spans = self
            .spans
            .into_iter()
            .filter_map(|mut s| {
                let start = s.range.start.saturating_sub(lead).min(keep);
                let end = s.range.end.saturating_sub(lead).min(keep);
                (start < end).then(|| {
                    s.range = start..end;
                    s
                })
            })
            .collect();
        self
    }

    /// No visible text.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// The link ranges, for click handling.
    pub fn links(&self) -> Vec<(Range<usize>, String)> {
        self.spans.iter().filter_map(|s| s.link.clone().map(|l| (s.range.clone(), l))).collect()
    }
}

/// A block of the reading pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A paragraph.
    Para(Inline),
    /// `h1`..`h6`.
    Heading(u8, Inline),
    /// A quotation.
    Quote(Vec<Block>),
    /// A list.
    List {
        /// Numbered.
        ordered: bool,
        /// Items.
        items: Vec<Vec<Block>>,
    },
    /// Preformatted text.
    Pre(String),
    /// A horizontal rule.
    Rule,
    /// An image (remote ones are placeholders).
    Image {
        /// `alt` text.
        alt: String,
        /// Where it comes from.
        source: ImageSource,
    },
    /// A data table: rows of cells of blocks.
    Table(Vec<Vec<Vec<Block>>>),
}

/// A message body laid out for native rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// The message.
    pub blocks: Vec<Block>,
    /// Quoted history, collapsed by default.
    pub quoted: Vec<Block>,
    /// The layout is beyond what the native renderer shows well; offer "Open original".
    pub complex: bool,
}

const MAX_NATIVE_TABLE_DEPTH: usize = 3;
const MAX_NATIVE_CELLS: usize = 120;
const MAX_NATIVE_IMAGES: usize = 16;

fn heading_level(tag: &str) -> Option<u8> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn inline_style(e: &SafeElement, mut s: Style) -> Style {
    match e.tag.as_str() {
        "b" | "strong" => s.bold = true,
        "i" | "em" | "cite" | "dfn" => s.italic = true,
        "u" | "ins" => s.underline = true,
        "s" | "strike" | "del" => s.strike = true,
        "code" | "kbd" | "samp" | "tt" => s.code = true,
        _ => {}
    }
    if let Some(w) = e.style_value("font-weight") {
        let w = w.trim_end_matches("!important").trim();
        if w == "bold" || w == "bolder" || w.parse::<u16>().is_ok_and(|n| n >= 600) {
            s.bold = true;
        }
    }
    if e.style_value("font-style").is_some_and(|v| v.contains("italic")) {
        s.italic = true;
    }
    if e.style_value("text-decoration").is_some_and(|v| v.contains("underline")) {
        s.underline = true;
    }
    s
}

/// Collapses HTML whitespace into single spaces.
fn collapse(text: &str, last_was_space: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = last_was_space;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
        } else {
            out.push(ch);
            space = false;
        }
    }
    out
}

struct Builder {
    blocks: Vec<Block>,
    inline: Inline,
}

impl Builder {
    fn new() -> Self {
        Self { blocks: Vec::new(), inline: Inline::default() }
    }

    fn flush(&mut self) {
        let inline = std::mem::take(&mut self.inline).trimmed();
        if !inline.is_blank() {
            self.blocks.push(Block::Para(inline));
        }
    }

    fn block(&mut self, b: Block) {
        self.flush();
        self.blocks.push(b);
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.blocks
    }

    fn nodes(&mut self, nodes: &[SafeNode], style: Style, link: Option<&str>) {
        for n in nodes {
            self.node(n, style, link);
        }
    }

    fn node(&mut self, n: &SafeNode, style: Style, link: Option<&str>) {
        match n {
            SafeNode::Stylesheet(_) => {}
            SafeNode::Text(t) => {
                let last_space = self.inline.text.is_empty() || self.inline.text.ends_with([' ', '\n']);
                let t = collapse(t, last_space);
                self.inline.push(&t, style, link);
            }
            SafeNode::Element(e) => self.element(e, style, link),
        }
    }

    fn element(&mut self, e: &SafeElement, style: Style, link: Option<&str>) {
        match e.display() {
            Display::Hidden => return,
            Display::LineBreak => {
                self.inline.push("\n", style, link);
                return;
            }
            _ => {}
        }
        let tag = e.tag.as_str();
        if let Some(level) = heading_level(tag) {
            let mut b = Builder::new();
            b.nodes(&e.children, Style { bold: true, ..style }, link);
            let inner = b.finish();
            self.flush();
            for block in inner {
                match block {
                    Block::Para(i) => self.blocks.push(Block::Heading(level, i)),
                    other => self.blocks.push(other),
                }
            }
            return;
        }
        match tag {
            "img" => {
                if let Some(source) = e.image.clone() {
                    let alt = e.attr("alt").unwrap_or("").to_owned();
                    self.block(Block::Image { alt, source });
                }
            }
            "hr" => self.block(Block::Rule),
            "pre" => {
                let text = SafeNode::Element(e.clone()).text_content();
                self.block(Block::Pre(text.trim_end().to_owned()));
            }
            "blockquote" => {
                let mut b = Builder::new();
                b.nodes(&e.children, style, link);
                let inner = b.finish();
                if !inner.is_empty() {
                    self.block(Block::Quote(inner));
                }
            }
            "ul" | "ol" => {
                let mut items = Vec::new();
                for c in &e.children {
                    match c {
                        SafeNode::Element(li) if li.tag == "li" => {
                            let mut b = Builder::new();
                            b.nodes(&li.children, style, link);
                            items.push(b.finish());
                        }
                        SafeNode::Text(t) if t.trim().is_empty() => {}
                        other => {
                            let mut b = Builder::new();
                            b.node(other, style, link);
                            let blocks = b.finish();
                            if !blocks.is_empty() {
                                items.push(blocks);
                            }
                        }
                    }
                }
                if !items.is_empty() {
                    self.block(Block::List { ordered: tag == "ol", items });
                }
            }
            "table" => {
                let rows = table_rows(e, style, link);
                let max_cols = rows.iter().map(Vec::len).max().unwrap_or(0);
                self.flush();
                if max_cols <= 1 {
                    // A layout table (one column): its cells read as plain blocks.
                    for row in rows {
                        for cell in row {
                            self.blocks.extend(cell);
                        }
                    }
                } else {
                    let rows: Vec<Vec<Vec<Block>>> =
                        rows.into_iter().filter(|r| r.iter().any(|c| !c.is_empty())).collect();
                    if !rows.is_empty() {
                        self.blocks.push(Block::Table(rows));
                    }
                }
            }
            "a" => {
                let target = e.href.as_deref().or(link);
                let s = if e.href.is_some() {
                    Style { underline: true, ..inline_style(e, style) }
                } else {
                    inline_style(e, style)
                };
                self.nodes(&e.children, s, target);
            }
            _ if e.display() == Display::Block || e.display() == Display::TablePart => {
                self.flush();
                self.nodes(&e.children, inline_style(e, style), link);
                self.flush();
            }
            _ => self.nodes(&e.children, inline_style(e, style), link),
        }
    }
}

/// Rows and cells of a table, walking through `tbody`/`thead`/`tfoot`.
fn table_rows(table: &SafeElement, style: Style, link: Option<&str>) -> Vec<Vec<Vec<Block>>> {
    let mut rows = Vec::new();
    fn walk(e: &SafeElement, style: Style, link: Option<&str>, rows: &mut Vec<Vec<Vec<Block>>>) {
        for c in &e.children {
            let SafeNode::Element(c) = c else { continue };
            match c.tag.as_str() {
                "thead" | "tbody" | "tfoot" => walk(c, style, link, rows),
                "tr" => {
                    let mut cells = Vec::new();
                    for cell in &c.children {
                        let SafeNode::Element(cell) = cell else { continue };
                        if cell.tag == "td" || cell.tag == "th" {
                            let mut b = Builder::new();
                            let s = if cell.tag == "th" { Style { bold: true, ..style } } else { style };
                            b.nodes(&cell.children, s, link);
                            cells.push(b.finish());
                        }
                    }
                    rows.push(cells);
                }
                "caption" => {
                    let mut b = Builder::new();
                    b.nodes(&c.children, Style { italic: true, ..style }, link);
                    rows.push(vec![b.finish()]);
                }
                _ => {}
            }
        }
    }
    walk(table, style, link, &mut rows);
    rows
}

fn contains_quote_start(e: &SafeElement) -> bool {
    e.quote_start || e.children.iter().any(|c| matches!(c, SafeNode::Element(x) if contains_quote_start(x)))
}

/// Splits the tree where quoted history starts (the element marked `quote_start`).
fn split_quote(nodes: &[SafeNode]) -> (Vec<SafeNode>, Vec<SafeNode>) {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut found = false;
    for n in nodes {
        if found {
            after.push(n.clone());
            continue;
        }
        match n {
            SafeNode::Element(e) if e.quote_start => {
                found = true;
                after.push(n.clone());
            }
            SafeNode::Element(e) if contains_quote_start(e) => {
                let (b, a) = split_quote(&e.children);
                before.push(SafeNode::Element(SafeElement { children: b, ..e.clone() }));
                after.push(SafeNode::Element(SafeElement { children: a, quote_start: false, ..e.clone() }));
                found = true;
            }
            other => before.push(other.clone()),
        }
    }
    (before, after)
}

#[derive(Default)]
struct Stats {
    table_depth: usize,
    max_table_depth: usize,
    cells: usize,
    images: usize,
}

fn stats(nodes: &[SafeNode], s: &mut Stats) {
    for n in nodes {
        let SafeNode::Element(e) = n else { continue };
        match e.tag.as_str() {
            "table" => {
                s.table_depth += 1;
                s.max_table_depth = s.max_table_depth.max(s.table_depth);
                stats(&e.children, s);
                s.table_depth -= 1;
                continue;
            }
            "td" | "th" => s.cells += 1,
            "img" => s.images += 1,
            _ => {}
        }
        stats(&e.children, s);
    }
}

/// Lays out a sanitized HTML tree.
pub fn layout_html(tree: &[SafeNode]) -> Layout {
    let mut s = Stats::default();
    stats(tree, &mut s);
    let complex =
        s.max_table_depth >= MAX_NATIVE_TABLE_DEPTH || s.cells > MAX_NATIVE_CELLS || s.images > MAX_NATIVE_IMAGES;
    let (before, after) = split_quote(tree);
    let mut b = Builder::new();
    b.nodes(&before, Style::default(), None);
    let mut q = Builder::new();
    q.nodes(&after, Style::default(), None);
    Layout { blocks: b.finish(), quoted: q.finish(), complex }
}

/// Finds `http(s)://` and `mailto:` links in plain text.
fn linkify(text: &str) -> Inline {
    let mut inline = Inline::default();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["https://", "http://", "mailto:"].iter().filter_map(|p| rest.find(p)).min();
        let Some(at) = next else {
            inline.push(rest, Style::default(), None);
            break;
        };
        inline.push(&rest[..at], Style::default(), None);
        let tail = &rest[at..];
        let end =
            tail.find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ')' | ']')).unwrap_or(tail.len());
        let mut url = &tail[..end];
        while url.ends_with(['.', ',', ';', ':', '!', '?']) {
            url = &url[..url.len() - 1];
        }
        if url.len() <= "https://".len() {
            inline.push(&tail[..1], Style::default(), None);
            rest = &tail[1..];
            continue;
        }
        inline.push(url, Style { underline: true, ..Style::default() }, Some(url));
        rest = &tail[url.len()..];
    }
    inline
}

fn text_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut quote: Vec<&str> = Vec::new();
    let flush_para = |para: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            let inline = linkify(&para.join("\n"));
            if !inline.is_blank() {
                blocks.push(Block::Para(inline));
            }
            para.clear();
        }
    };
    let flush_quote = |quote: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !quote.is_empty() {
            let inner = text_blocks(&quote.join("\n"));
            if !inner.is_empty() {
                blocks.push(Block::Quote(inner));
            }
            quote.clear();
        }
    };
    for line in text.lines() {
        let trimmed = line.trim_end_matches('\r');
        if let Some(q) = trimmed.strip_prefix('>') {
            flush_para(&mut para, &mut blocks);
            quote.push(q.strip_prefix(' ').unwrap_or(q));
        } else if trimmed.trim().is_empty() {
            flush_para(&mut para, &mut blocks);
            flush_quote(&mut quote, &mut blocks);
        } else {
            flush_quote(&mut quote, &mut blocks);
            para.push(trimmed);
        }
    }
    flush_para(&mut para, &mut blocks);
    flush_quote(&mut quote, &mut blocks);
    blocks
}

/// Lays out a plain-text body, collapsing quoted history from `quote_start`.
pub fn layout_text(text: &str, quote_start: Option<usize>) -> Layout {
    let at = quote_start.filter(|&i| i <= text.len() && text.is_char_boundary(i)).unwrap_or(text.len());
    Layout { blocks: text_blocks(&text[..at]), quoted: text_blocks(&text[at..]), complex: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_mime::{RemoteImagePolicy, SanitizeOptions, sanitize_html};

    fn layout(html: &str) -> Layout {
        let s = sanitize_html(
            html,
            &SanitizeOptions { remote_images: RemoteImagePolicy::Original, ..SanitizeOptions::default() },
        );
        layout_html(&s.tree)
    }

    fn para(b: &Block) -> &Inline {
        match b {
            Block::Para(i) | Block::Heading(_, i) => i,
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn paragraphs_runs_and_links() {
        let l =
            layout("<p>Hello <b>bold</b> and <a href=\"https://example.com/x\">a <i>link</i></a>.</p><p>Second</p>");
        assert_eq!(l.blocks.len(), 2);
        let p = para(&l.blocks[0]);
        assert_eq!(p.text, "Hello bold and a link.");
        assert_eq!(&p.text[p.spans[0].range.clone()], "bold");
        assert!(p.spans[0].style.bold);
        let links = p.links();
        assert_eq!(links.len(), 2, "the italic part of the link is its own run");
        assert!(links.iter().all(|(_, href)| href == "https://example.com/x"));
        assert_eq!(&p.text[links[0].0.start..links[1].0.end], "a link");
    }

    #[test]
    fn scripts_hidden_text_and_styles_never_render() {
        let l = layout(
            "<style>p{color:red}</style><div style=\"display:none\">preheader</div><script>alert(1)</script>\
             <p onclick=\"x()\">Visible</p>",
        );
        assert_eq!(l.blocks, vec![Block::Para(Inline { text: "Visible".into(), spans: vec![] })]);
    }

    #[test]
    fn whitespace_collapses_and_br_breaks_lines() {
        let l = layout("<div>  one\n\n   two<br>three  </div>");
        assert_eq!(para(&l.blocks[0]).text, "one two\nthree");
    }

    #[test]
    fn lists_quotes_headings_pre_and_rules() {
        let l = layout(
            "<h2>Title</h2><ol><li>a</li><li>b <b>c</b></li></ol><blockquote><p>q</p></blockquote><pre>  x\n  y</pre><hr>",
        );
        assert!(matches!(&l.blocks[0], Block::Heading(2, i) if i.text == "Title" && i.spans[0].style.bold));
        assert!(matches!(&l.blocks[1], Block::List { ordered: true, items } if items.len() == 2));
        assert!(matches!(&l.blocks[2], Block::Quote(inner) if inner.len() == 1));
        assert_eq!(l.blocks[3], Block::Pre("  x\n  y".into()));
        assert_eq!(l.blocks[4], Block::Rule);
    }

    #[test]
    fn layout_tables_flatten_and_data_tables_stay() {
        let l = layout("<table><tr><td><p>A</p></td></tr><tr><td><p>B</p></td></tr></table>");
        assert_eq!(l.blocks.len(), 2, "one-column tables read as blocks");
        let l = layout("<table><tr><th>K</th><th>V</th></tr><tr><td>a</td><td>1</td></tr></table>");
        let Block::Table(rows) = &l.blocks[0] else { panic!("expected a table") };
        assert_eq!(rows.len(), 2);
        assert!(para(&rows[0][0][0]).spans[0].style.bold, "header cells are bold");
    }

    #[test]
    fn remote_images_are_placeholders_and_trackers_are_gone() {
        let l = layout(
            "<p>x</p><img src=\"https://cdn.example/a.png\" alt=\"Logo\"><img src=\"https://t.example/p.gif\" width=1 height=1>",
        );
        let images: Vec<_> = l.blocks.iter().filter(|b| matches!(b, Block::Image { .. })).collect();
        assert_eq!(images.len(), 1);
        assert!(
            matches!(images[0], Block::Image { alt, source: ImageSource::Remote { url } } if alt == "Logo" && url.contains("cdn.example"))
        );
    }

    #[test]
    fn quoted_history_is_split_off() {
        let html = "<div><p>New text</p><div class=\"gmail_quote\"><p>On Mon, Ada wrote:</p><blockquote><p>old</p></blockquote></div></div>";
        let l = layout(html);
        assert_eq!(para(&l.blocks[0]).text, "New text");
        assert!(l.blocks.len() == 1, "{:?}", l.blocks);
        assert!(!l.quoted.is_empty());
    }

    #[test]
    fn deeply_nested_tables_are_complex() {
        let nested = "<table><tr><td><table><tr><td><table><tr><td>x</td><td>y</td></tr></table></td></tr></table></td></tr></table>";
        assert!(layout(nested).complex);
        assert!(!layout("<p>simple</p>").complex);
    }

    #[test]
    fn plain_text_paragraphs_quotes_and_links() {
        let text = "Hi Ada,\n\nSee https://example.com/doc. Thanks!\n\nOn Mon, Ada wrote:\n> old line\n> more";
        let at = akatsuki_mime::quote::text_quote_start(text);
        let l = layout_text(text, at);
        assert_eq!(l.blocks.len(), 2);
        let p = para(&l.blocks[1]);
        assert_eq!(p.links(), vec![(4..27, "https://example.com/doc".to_owned())]);
        assert!(matches!(&l.quoted[..], [Block::Para(_), Block::Quote(_)]));
        let l = layout_text("> a\n> b\nreply", None);
        assert!(matches!(&l.blocks[..], [Block::Quote(_), Block::Para(_)]));
    }
}
