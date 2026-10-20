//! Keyboard dispatch: the Rust twin of `createMatcher` in `packages/shortcuts/src/index.ts`, so
//! keys behave exactly as on the web (D-315). Bindings come from `crates/shortcuts`
//! (`packages/shortcuts/shortcuts.json`), in the "+" notation the JSON uses; nothing here
//! hard-codes a key.
//!
//! - A combo is a key plus modifiers. Shift is dropped for symbols typed without a command
//!   modifier (`#`, `?`), because it is part of the character.
//! - Sequences (`g i`) wait at most [`SEQUENCE_TIMEOUT_MS`] between steps.
//! - Global single keys are off while a text field has focus (compose, search, palette); global
//!   command combos (`ctrl+k`) still fire there.

use std::collections::HashSet;

use akatsuki_shortcuts::{Keymap, Platform, SHORTCUTS, Scope, Shortcut};

/// Max gap between the steps of a sequence (same default as the web).
pub const SEQUENCE_TIMEOUT_MS: i64 = 1200;

/// Scopes whose focus is a text field.
pub const TEXT_SCOPES: [Scope; 3] = [Scope::Compose, Scope::Search, Scope::Palette];

const NAMED_KEYS: [&str; 10] =
    ["enter", "escape", "tab", "space", "backspace", "delete", "up", "down", "left", "right"];

/// One key press.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Combo {
    /// Lowercase key or a named key.
    pub key: String,
    /// Cmd (macOS).
    pub cmd: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

impl Combo {
    /// Cmd, Ctrl or Alt is held.
    pub fn has_command_modifier(&self) -> bool {
        self.cmd || self.ctrl || self.alt
    }
}

/// A parse error in the key map (a bug in `shortcuts.json`, caught by tests).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct KeyParseError(String);

/// Parses `"ctrl+shift+,"`, `"#"`, `"+"`.
pub fn parse_combo(input: &str) -> Result<Combo, KeyParseError> {
    if input.is_empty() {
        return Err(KeyParseError("empty combo".into()));
    }
    let parts: Vec<String> = if input == "+" {
        vec!["+".into()]
    } else if let Some(head) = input.strip_suffix("++") {
        head.split('+').map(str::to_owned).chain(["+".to_owned()]).collect()
    } else {
        input.split('+').map(str::to_owned).collect()
    };
    let mut combo = Combo::default();
    let last = parts.len() - 1;
    for (i, raw) in parts.iter().enumerate() {
        let part = if raw.chars().count() == 1 { raw.clone() } else { raw.to_lowercase() };
        if i < last {
            let flag = match part.as_str() {
                "cmd" => &mut combo.cmd,
                "ctrl" => &mut combo.ctrl,
                "alt" => &mut combo.alt,
                "shift" => &mut combo.shift,
                _ => return Err(KeyParseError(format!("unknown modifier \"{raw}\" in \"{input}\""))),
            };
            if *flag {
                return Err(KeyParseError(format!("duplicate modifier \"{raw}\" in \"{input}\"")));
            }
            *flag = true;
            continue;
        }
        if part.is_empty() {
            return Err(KeyParseError(format!("missing key in \"{input}\"")));
        }
        if part.chars().count() > 1 && !NAMED_KEYS.contains(&part.as_str()) {
            return Err(KeyParseError(format!("unknown key \"{raw}\" in \"{input}\"")));
        }
        combo.key = if part.chars().count() == 1 { part.to_lowercase() } else { part };
    }
    Ok(combo)
}

/// Parses `"g i"`.
pub fn parse_sequence(input: &str) -> Result<Vec<Combo>, KeyParseError> {
    let steps: Vec<&str> = input.split(' ').filter(|s| !s.is_empty()).collect();
    if steps.is_empty() {
        return Err(KeyParseError("empty key sequence".into()));
    }
    steps.into_iter().map(parse_combo).collect()
}

/// The unshifted key of a US-layout symbol, for combos with a command modifier (the web reads the
/// physical key there; GPUI on Linux reports the shifted symbol, e.g. `<` for Ctrl+Shift+,).
fn unshifted(symbol: &str) -> Option<&'static str> {
    Some(match symbol {
        "<" => ",",
        ">" => ".",
        "?" => "/",
        ":" => ";",
        "\"" => "'",
        "{" => "[",
        "}" => "]",
        "|" => "\\",
        "~" => "`",
        "_" => "-",
        "+" => "=",
        "!" => "1",
        "@" => "2",
        "#" => "3",
        "$" => "4",
        "%" => "5",
        "^" => "6",
        "&" => "7",
        "*" => "8",
        "(" => "9",
        ")" => "0",
        _ => return None,
    })
}

fn is_symbol(key: &str) -> bool {
    let mut chars = key.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_ascii_alphanumeric())
}

/// Normalizes a GPUI keystroke (`key`, modifiers; `platform` is Cmd on macOS) into a combo, with
/// the same rules as `comboFromEvent` on the web.
pub fn combo_from_keystroke(key: &str, ctrl: bool, alt: bool, shift: bool, platform: bool) -> Combo {
    let cmd = cfg!(target_os = "macos") && platform;
    let command = cmd || ctrl || alt;
    let key = key.to_lowercase();
    if command && let Some(base) = unshifted(&key) {
        return Combo { key: base.to_owned(), cmd, ctrl, alt, shift: true };
    }
    Combo { shift: if !command && is_symbol(&key) { false } else { shift }, key, cmd, ctrl, alt }
}

/// Whether a shortcut of `scope` may fire while `active` scopes are on (`scopeApplies`).
pub fn scope_applies(scope: Scope, active: &HashSet<Scope>, first: &Combo) -> bool {
    let in_text = TEXT_SCOPES.iter().any(|s| active.contains(s));
    match scope {
        Scope::Global => !in_text || first.has_command_modifier(),
        Scope::Mail => {
            active.contains(&Scope::List) || active.contains(&Scope::Thread) || active.contains(&Scope::Mail)
        }
        other => active.contains(&other),
    }
}

/// The key strings of a shortcut on a platform and key map.
pub fn keys_for(s: &Shortcut, platform: Platform, keymap: Keymap) -> &'static [&'static str] {
    s.keys_for(keymap).for_platform(platform)
}

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Shortcut id.
    pub id: &'static str,
    /// Scope.
    pub scope: Scope,
    /// Steps.
    pub sequence: Vec<Combo>,
}

/// Every binding of a key map on a platform.
pub fn bindings_for(platform: Platform, keymap: Keymap) -> Vec<Binding> {
    SHORTCUTS
        .iter()
        .flat_map(|s| {
            keys_for(s, platform, keymap).iter().filter_map(move |k| {
                parse_sequence(k).ok().map(|sequence| Binding { id: s.id, scope: s.scope, sequence })
            })
        })
        .collect()
}

/// The matcher's verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatchResult {
    /// Run this shortcut.
    Match(&'static str),
    /// The keys so far start a sequence; swallow the key and wait.
    Pending,
    /// Not a shortcut; let the key through.
    None,
}

/// Stateful matcher for single combos and multi-step sequences.
#[derive(Clone, Debug)]
pub struct Matcher {
    bindings: Vec<Binding>,
    pending: Vec<Combo>,
    last_at: i64,
}

impl Matcher {
    /// A matcher for this platform and key map.
    pub fn new(platform: Platform, keymap: Keymap) -> Self {
        Self { bindings: bindings_for(platform, keymap), pending: Vec::new(), last_at: 0 }
    }

    /// Forgets a half-typed sequence.
    pub fn reset(&mut self) {
        self.pending.clear();
    }

    /// A sequence is half typed.
    #[cfg(test)]
    pub fn is_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Feeds one key press at `now_ms`.
    pub fn handle(&mut self, combo: Combo, active: &HashSet<Scope>, now_ms: i64) -> MatchResult {
        if ["shift", "control", "ctrl", "meta", "alt", "cmd", "platform", "super", "function"]
            .contains(&combo.key.as_str())
        {
            return MatchResult::None;
        }
        if !self.pending.is_empty() && now_ms - self.last_at > SEQUENCE_TIMEOUT_MS {
            self.pending.clear();
        }
        self.last_at = now_ms;
        let mut typed = self.pending.clone();
        typed.push(combo);
        let mut prefix = false;
        for b in &self.bindings {
            if !scope_applies(b.scope, active, &b.sequence[0]) || b.sequence.len() < typed.len() {
                continue;
            }
            if !typed.iter().zip(&b.sequence).all(|(a, b)| a == b) {
                continue;
            }
            if b.sequence.len() == typed.len() {
                self.pending.clear();
                return MatchResult::Match(b.id);
            }
            prefix = true;
        }
        if prefix {
            self.pending = typed;
            return MatchResult::Pending;
        }
        self.pending.clear();
        MatchResult::None
    }
}

/// Human label: `Ctrl+K` on Linux and Windows, `⌘K` on macOS, `G then I` for sequences
/// (`formatKeys`).
pub fn format_keys(keys: &str, platform: Platform) -> String {
    let Ok(seq) = parse_sequence(keys) else { return keys.to_owned() };
    seq.iter()
        .map(|c| {
            let key = match c.key.as_str() {
                "enter" => "Enter".to_owned(),
                "escape" => "Esc".to_owned(),
                "tab" => "Tab".to_owned(),
                "space" => "Space".to_owned(),
                "backspace" => "Backspace".to_owned(),
                "delete" => "Delete".to_owned(),
                "up" => "↑".to_owned(),
                "down" => "↓".to_owned(),
                "left" => "←".to_owned(),
                "right" => "→".to_owned(),
                k => k.to_uppercase(),
            };
            if platform == Platform::Mac {
                let mut s = String::new();
                for (on, sym) in [(c.ctrl, "⌃"), (c.alt, "⌥"), (c.shift, "⇧"), (c.cmd, "⌘")] {
                    if on {
                        s.push_str(sym);
                    }
                }
                s + &key
            } else {
                let mut parts: Vec<String> = Vec::new();
                for (on, name) in [(c.cmd, "Win"), (c.ctrl, "Ctrl"), (c.alt, "Alt"), (c.shift, "Shift")] {
                    if on {
                        parts.push(name.to_owned());
                    }
                }
                parts.push(key);
                parts.join("+")
            }
        })
        .collect::<Vec<_>>()
        .join(" then ")
}

/// Keycaps of a binding, one per key: `ctrl+shift+l` → `Ctrl`, `Shift`, `L`; `g i` → `G`, `I`;
/// `cmd+k` on macOS → `⌘`, `K`. Drawn as separate kbds (First Light: no "then").
pub fn keycaps(keys: &str, platform: Platform) -> Vec<String> {
    let Ok(seq) = parse_sequence(keys) else { return vec![keys.to_owned()] };
    let mut out = Vec::new();
    for c in seq {
        let mac = platform == Platform::Mac;
        for (on, name, sym) in
            [(c.ctrl, "Ctrl", "⌃"), (c.alt, "Alt", "⌥"), (c.shift, "Shift", "⇧"), (c.cmd, "Win", "⌘")]
        {
            if on {
                out.push(if mac { sym } else { name }.to_owned());
            }
        }
        out.push(match c.key.as_str() {
            "enter" => "↵".to_owned(),
            "escape" => "Esc".to_owned(),
            "tab" => "Tab".to_owned(),
            "space" => "Space".to_owned(),
            "backspace" => "⌫".to_owned(),
            "delete" => "Del".to_owned(),
            "up" => "↑".to_owned(),
            "down" => "↓".to_owned(),
            "left" => "←".to_owned(),
            "right" => "→".to_owned(),
            k => k.to_uppercase(),
        });
    }
    out
}

/// The keycaps of a shortcut's first binding on this platform (empty when it has no key).
pub fn caps(id: &str, keymap: Keymap) -> Vec<String> {
    let platform = Platform::current();
    akatsuki_shortcuts::find(id)
        .and_then(|s| keys_for(s, platform, keymap).first().map(|k| keycaps(k, platform)))
        .unwrap_or_default()
}

/// The first key hint of a shortcut, formatted for this platform (palette and tooltips).
pub fn hint(id: &str, keymap: Keymap) -> Option<String> {
    let s = akatsuki_shortcuts::find(id)?;
    let platform = Platform::current();
    keys_for(s, platform, keymap).first().map(|k| format_keys(k, platform))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycaps_split_combos_and_sequences() {
        assert_eq!(keycaps("g i", Platform::Linux), ["G", "I"]);
        assert_eq!(keycaps("ctrl+shift+l", Platform::Linux), ["Ctrl", "Shift", "L"]);
        assert_eq!(keycaps("cmd+k", Platform::Mac), ["⌘", "K"]);
        assert_eq!(keycaps("ctrl+enter", Platform::Windows), ["Ctrl", "↵"]);
        assert_eq!(keycaps("#", Platform::Linux), ["#"]);
    }

    fn scopes(s: &[Scope]) -> HashSet<Scope> {
        s.iter().copied().collect()
    }

    fn key(k: &str) -> Combo {
        combo_from_keystroke(k, false, false, false, false)
    }

    fn list() -> HashSet<Scope> {
        scopes(&[Scope::Global, Scope::Mail, Scope::List])
    }

    #[test]
    fn every_binding_in_the_key_map_parses() {
        for platform in [Platform::Linux, Platform::Mac, Platform::Windows] {
            for keymap in [Keymap::Superhuman, Keymap::Gmail] {
                for s in SHORTCUTS {
                    for k in keys_for(s, platform, keymap) {
                        parse_sequence(k).unwrap_or_else(|e| panic!("{}: {e}", s.id));
                    }
                }
            }
        }
    }

    #[test]
    fn parses_modifiers_named_keys_and_plus() {
        assert_eq!(
            parse_combo("ctrl+shift+,").unwrap(),
            Combo { key: ",".into(), ctrl: true, shift: true, ..Combo::default() }
        );
        assert_eq!(parse_combo("+").unwrap().key, "+");
        assert_eq!(parse_combo("ctrl++").unwrap(), Combo { key: "+".into(), ctrl: true, ..Combo::default() });
        assert_eq!(parse_combo("Enter").unwrap().key, "enter");
        assert!(parse_combo("hyper+k").is_err());
        assert!(parse_combo("ctrl+ctrl+k").is_err());
        assert!(parse_combo("pageup").is_err());
        assert_eq!(parse_sequence("g  i").unwrap().len(), 2);
    }

    #[test]
    fn single_keys_and_sequences() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        assert_eq!(m.handle(key("e"), &list(), 0), MatchResult::Match("thread.archive"));
        assert_eq!(m.handle(key("j"), &list(), 10), MatchResult::Match("nav.next"));
        assert_eq!(m.handle(key("down"), &list(), 20), MatchResult::Match("nav.next"));
        assert_eq!(m.handle(key("g"), &list(), 30), MatchResult::Pending);
        assert!(m.is_pending());
        assert_eq!(m.handle(key("i"), &list(), 40), MatchResult::Match("go.inbox"));
        assert_eq!(m.handle(key("g"), &list(), 50), MatchResult::Pending);
        assert_eq!(m.handle(key("q"), &list(), 60), MatchResult::None, "a wrong second step cancels");
        assert_eq!(m.handle(key("e"), &list(), 70), MatchResult::Match("thread.archive"));
    }

    #[test]
    fn sequences_time_out() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        assert_eq!(m.handle(key("g"), &list(), 0), MatchResult::Pending);
        assert_eq!(m.handle(key("i"), &list(), SEQUENCE_TIMEOUT_MS + 1), MatchResult::None);
    }

    #[test]
    fn symbols_drop_shift_and_command_combos_use_the_base_key() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        // Shift+3 arrives as "#" (GPUI already drops shift for symbols).
        assert_eq!(
            m.handle(combo_from_keystroke("#", false, false, true, false), &list(), 0),
            MatchResult::Match("thread.trash")
        );
        assert_eq!(
            m.handle(combo_from_keystroke("?", false, false, true, false), &list(), 1),
            MatchResult::Match("help.shortcuts")
        );
        // Ctrl+Shift+, arrives as "ctrl-<".
        let compose = scopes(&[Scope::Global, Scope::Compose]);
        assert_eq!(
            m.handle(combo_from_keystroke("<", true, false, false, false), &compose, 2),
            MatchResult::Match("compose.discard")
        );
        // Shift+J is a selection extension, not "j".
        assert_eq!(
            m.handle(combo_from_keystroke("j", false, false, true, false), &list(), 3),
            MatchResult::Match("select.extendDown")
        );
        assert_eq!(
            m.handle(combo_from_keystroke("tab", false, false, true, false), &list(), 4),
            MatchResult::Match("split.prev")
        );
    }

    #[test]
    fn text_scopes_block_single_keys_but_not_command_combos() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        let compose = scopes(&[Scope::Global, Scope::Compose]);
        assert_eq!(m.handle(key("c"), &compose, 0), MatchResult::None, "typing a c in a draft");
        assert_eq!(m.handle(key("g"), &compose, 1), MatchResult::None);
        assert_eq!(
            m.handle(combo_from_keystroke("k", true, false, false, false), &compose, 2),
            MatchResult::Match("palette.open")
        );
        assert_eq!(
            m.handle(combo_from_keystroke("enter", true, false, false, false), &compose, 3),
            MatchResult::Match("compose.send")
        );
        assert_eq!(m.handle(key("escape"), &compose, 4), MatchResult::Match("compose.close"));
        let search = scopes(&[Scope::Global, Scope::Search]);
        assert_eq!(m.handle(key("e"), &search, 5), MatchResult::None);
        assert_eq!(m.handle(key("enter"), &search, 6), MatchResult::Match("search.submit"));
    }

    #[test]
    fn mail_scope_needs_a_list_or_thread() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        assert_eq!(m.handle(key("e"), &scopes(&[Scope::Global]), 0), MatchResult::None);
        let thread = scopes(&[Scope::Global, Scope::Mail, Scope::Thread]);
        assert_eq!(m.handle(key("e"), &thread, 1), MatchResult::Match("thread.archive"));
        assert_eq!(m.handle(key("x"), &thread, 2), MatchResult::None, "select.toggle is list-only");
        assert_eq!(
            m.handle(combo_from_keystroke("x", false, false, true, false), &thread, 3),
            MatchResult::Match("thread.expandAll")
        );
    }

    #[test]
    fn gmail_keymap_overrides() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Gmail);
        assert_eq!(m.handle(key("b"), &list(), 0), MatchResult::Match("thread.snooze"));
        assert_eq!(m.handle(key("h"), &list(), 1), MatchResult::None);
        assert_eq!(m.handle(key("u"), &list(), 2), MatchResult::Match("nav.back"));
        assert_eq!(
            m.handle(combo_from_keystroke("i", false, false, true, false), &list(), 3),
            MatchResult::Match("thread.toggleRead")
        );
        let mut s = Matcher::new(Platform::Linux, Keymap::Superhuman);
        assert_eq!(s.handle(key("u"), &list(), 0), MatchResult::Match("thread.toggleRead"));
    }

    #[test]
    fn modifier_keys_alone_do_nothing() {
        let mut m = Matcher::new(Platform::Linux, Keymap::Superhuman);
        assert_eq!(m.handle(key("g"), &list(), 0), MatchResult::Pending);
        assert_eq!(m.handle(key("shift"), &list(), 1), MatchResult::None);
        assert!(m.is_pending(), "a lone modifier keeps the sequence");
    }

    #[test]
    fn formats_like_the_web() {
        assert_eq!(format_keys("ctrl+shift+,", Platform::Linux), "Ctrl+Shift+,");
        assert_eq!(format_keys("cmd+k", Platform::Mac), "⌘K");
        assert_eq!(format_keys("g i", Platform::Linux), "G then I");
        assert_eq!(format_keys("shift+tab", Platform::Windows), "Shift+Tab");
        assert_eq!(hint("thread.archive", Keymap::Superhuman).as_deref(), Some("E"));
        assert_eq!(hint("thread.openOriginal", Keymap::Superhuman), None, "palette-only");
    }
}
