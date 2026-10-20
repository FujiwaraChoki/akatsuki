//! Quoted-history detection for plain text (`quoted.textStart`, ARCHITECTURE §7.3). HTML quotes
//! are marked by the sanitizer.

fn is_attribution(line: &str) -> bool {
    let l = line.trim();
    let lower = l.to_lowercase();
    (lower.starts_with("on ") && lower.ends_with("wrote:"))
        || (lower.starts_with("am ") && lower.ends_with("schrieb:"))
        || (lower.starts_with("le ") && (lower.ends_with("a écrit :") || lower.ends_with("a écrit:")))
        || (lower.starts_with("el ") && lower.ends_with("escribió:"))
}

fn is_separator(line: &str) -> bool {
    let l = line.trim();
    let lower = l.to_lowercase();
    (l.starts_with("-----")
        && l.ends_with("-----")
        && (lower.contains("original") || lower.contains("ursprüngliche") || lower.contains("forwarded")))
        || (l.len() >= 20 && l.chars().all(|c| c == '_'))
}

/// Byte offset in `text` where quoted history starts, if any.
///
/// Recognised, earliest first: an attribution line (`On … wrote:`, also wrapped over two lines and
/// in German, French and Spanish) followed by `>` lines; an Outlook separator (`-----Original
/// Message-----` or a line of underscores) followed by a `From:` header block; or a trailing block
/// where every remaining non-empty line starts with `>`. Inline (interleaved) replies are not
/// collapsed.
pub fn text_quote_start(text: &str) -> Option<usize> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        lines.push((offset, line.trim_end_matches(['\r', '\n'])));
        offset += line.len();
    }
    let next_content = |from: usize| lines.iter().skip(from).find(|(_, l)| !l.trim().is_empty()).map(|(_, l)| *l);

    for (i, &(off, line)) in lines.iter().enumerate() {
        if is_attribution(line) && next_content(i + 1).is_some_and(|l| l.trim_start().starts_with('>')) {
            return Some(off);
        }
        if i + 1 < lines.len() {
            let joined = format!("{} {}", line.trim(), lines[i + 1].1.trim());
            if line.trim().to_lowercase().starts_with("on ")
                && is_attribution(&joined)
                && next_content(i + 2).is_some_and(|l| l.trim_start().starts_with('>'))
            {
                return Some(off);
            }
        }
        if is_separator(line) && next_content(i + 1).is_some_and(|l| l.trim_start().to_lowercase().starts_with("from:"))
        {
            return Some(off);
        }
    }
    // Trailing `>` block.
    let first_quoted = lines.iter().position(|(_, l)| l.trim_start().starts_with('>'))?;
    let all_quoted_after =
        lines[first_quoted..].iter().all(|(_, l)| l.trim().is_empty() || l.trim_start().starts_with('>'));
    if all_quoted_after && first_quoted > 0 {
        return Some(lines[first_quoted].0);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_then_quote() {
        let t = "Sounds good.\n\nOn Mon, 19 Oct 2026 at 10:00, Ada <ada@x.com> wrote:\n> Lunch?\n> > Earlier\n";
        let at = text_quote_start(t).unwrap();
        assert!(t[at..].starts_with("On Mon"));
    }

    #[test]
    fn wrapped_attribution() {
        let t = "Yes\n\nOn Mon, 19 Oct 2026 at 10:00, A Very Long Name <someone@example.com>\nwrote:\n\n> hi\n";
        assert!(t[text_quote_start(t).unwrap()..].starts_with("On Mon"));
    }

    #[test]
    fn outlook_separator() {
        let t = "Thanks\r\n\r\n-----Original Message-----\r\nFrom: Bob\r\nSent: Monday\r\n";
        assert!(t[text_quote_start(t).unwrap()..].starts_with("-----Original"));
        let t = "Thanks\n________________________________\nFrom: Bob <b@x>\nSent: Monday\n";
        assert!(t[text_quote_start(t).unwrap()..].starts_with("____"));
    }

    #[test]
    fn interleaved_replies_are_not_collapsed() {
        assert_eq!(text_quote_start("> question one\nanswer one\n> question two\nanswer two\n"), None);
        assert_eq!(text_quote_start("no quotes here\n"), None);
        assert_eq!(text_quote_start("> entire message is a quote\n"), None);
        let t = "reply\n> quoted\n>\n> more\n";
        assert_eq!(text_quote_start(t), Some(6));
    }

    #[test]
    fn localized() {
        let t = "Ok\nAm 19.10.2026 um 10:00 schrieb Ada <a@x>:\n> Hallo\n";
        assert!(text_quote_start(t).is_some());
    }
}
