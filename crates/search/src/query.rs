//! The search query grammar (ARCHITECTURE §11.1). One grammar, two implementations: this module
//! and the TypeScript parser (`convex/lib/query.ts` until `packages/mail-model/src/query/` exists,
//! D-126); both must pass `packages/mail-model/fixtures/query.json` (ASTs, normalized text and
//! error messages).
//!
//! ```text
//! query    = ws* or_expr? ws*
//! or_expr  = and_expr ( ws+ "OR" ws+ and_expr )*
//! and_expr = unary ( ws+ unary )*
//! unary    = [ "-" ] atom
//! atom     = "(" or_expr ")" | field ":" value | phrase | word
//! field    = from | to | cc | subject | label | in | is | has | before | after
//!          | older_than | newer_than | account | list | filename | category      ; case-insensitive
//! value    = phrase | word
//! phrase   = DQUOTE *( any except DQUOTE and "\" / "\" DQUOTE / "\\" ) DQUOTE
//! word     = 1*( any char except ws, DQUOTE, "(", ")" )      ; unknown "x:y" → word "x:y"
//! ```
//!
//! Resolution rules this implementation fixes (the TypeScript twin follows them):
//!
//! - `before:` / `after:` take `YYYY-MM-DD` or `YYYY/MM/DD` and resolve to the start of that day in
//!   the given IANA time zone (`before:` means `date < at`, `after:` means `date >= at`).
//! - `older_than:` / `newer_than:` take `N(d|w|m|y)` and resolve to days: `d`=1, `w`=7, `m`=30,
//!   `y`=365.
//! - `is:`, `has:`, `in:` (for the built-in folders) and `category:` values are lowercased; other
//!   values keep their case. Unknown values produce an error but stay in the AST.
//! - A malformed value (bad date, empty value) produces an error and the whole token becomes a
//!   text word, so nothing the user typed is silently dropped.
//! - A one-element `and`/`or` collapses to its element; an empty query is `{"and": []}`.

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Fields whose value is a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// `from:`
    From,
    /// `to:`
    To,
    /// `cc:`
    Cc,
    /// `subject:`
    Subject,
    /// `label:`
    Label,
    /// `in:`
    In,
    /// `is:`
    Is,
    /// `has:`
    Has,
    /// `list:`
    List,
    /// `filename:`
    Filename,
    /// `account:`
    Account,
    /// `category:`
    Category,
}

impl Field {
    /// The keyword.
    pub fn as_str(self) -> &'static str {
        match self {
            Field::From => "from",
            Field::To => "to",
            Field::Cc => "cc",
            Field::Subject => "subject",
            Field::Label => "label",
            Field::In => "in",
            Field::Is => "is",
            Field::Has => "has",
            Field::List => "list",
            Field::Filename => "filename",
            Field::Account => "account",
            Field::Category => "category",
        }
    }
}

/// `before:` / `after:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateField {
    /// `date < at`.
    Before,
    /// `date >= at`.
    After,
}

/// `older_than:` / `newer_than:`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgeField {
    /// Older than N days.
    OlderThan,
    /// Newer than N days.
    NewerThan,
}

/// `QueryAst`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryAst {
    /// `{ and: QueryAst[] }`
    And(Vec<QueryAst>),
    /// `{ or: QueryAst[] }`
    Or(Vec<QueryAst>),
    /// `{ not: QueryAst }`
    Not(Box<QueryAst>),
    /// `{ text, phrase }`
    Text {
        /// The word or phrase.
        text: String,
        /// Quoted phrase.
        phrase: bool,
    },
    /// `{ field, value }`
    Field {
        /// Field.
        field: Field,
        /// Value.
        value: String,
    },
    /// `{ field: "before" | "after", at }`
    Date {
        /// Field.
        field: DateField,
        /// Resolved instant (ms).
        at: i64,
    },
    /// `{ field: "older_than" | "newer_than", days }`
    Age {
        /// Field.
        field: AgeField,
        /// Days.
        days: u32,
    },
}

/// The name the contract uses (`search::query::Ast`).
pub type Ast = QueryAst;

impl QueryAst {
    /// The empty query.
    pub fn empty() -> Self {
        QueryAst::And(Vec::new())
    }

    /// `true` for the empty query.
    pub fn is_empty(&self) -> bool {
        matches!(self, QueryAst::And(v) if v.is_empty())
    }
}

impl Serialize for QueryAst {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        match self {
            QueryAst::And(v) => m.serialize_entry("and", v)?,
            QueryAst::Or(v) => m.serialize_entry("or", v)?,
            QueryAst::Not(v) => m.serialize_entry("not", v)?,
            QueryAst::Text { text, phrase } => {
                m.serialize_entry("text", text)?;
                m.serialize_entry("phrase", phrase)?;
            }
            QueryAst::Field { field, value } => {
                m.serialize_entry("field", field)?;
                m.serialize_entry("value", value)?;
            }
            QueryAst::Date { field, at } => {
                m.serialize_entry("field", field)?;
                m.serialize_entry("at", at)?;
            }
            QueryAst::Age { field, days } => {
                m.serialize_entry("field", field)?;
                m.serialize_entry("days", days)?;
            }
        }
        m.end()
    }
}

impl<'de> Deserialize<'de> for QueryAst {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        from_json(&v).map_err(D::Error::custom)
    }
}

fn from_json(v: &serde_json::Value) -> Result<QueryAst, String> {
    let obj = v.as_object().ok_or("QueryAst must be an object")?;
    let list = |x: &serde_json::Value| -> Result<Vec<QueryAst>, String> {
        x.as_array().ok_or("expected array")?.iter().map(from_json).collect()
    };
    if let Some(x) = obj.get("and") {
        return Ok(QueryAst::And(list(x)?));
    }
    if let Some(x) = obj.get("or") {
        return Ok(QueryAst::Or(list(x)?));
    }
    if let Some(x) = obj.get("not") {
        return Ok(QueryAst::Not(Box::new(from_json(x)?)));
    }
    if let Some(text) = obj.get("text") {
        let text = text.as_str().ok_or("text must be a string")?.to_owned();
        let phrase = obj.get("phrase").and_then(serde_json::Value::as_bool).unwrap_or(false);
        return Ok(QueryAst::Text { text, phrase });
    }
    let field = obj.get("field").and_then(serde_json::Value::as_str).ok_or("missing field")?;
    match field {
        "before" | "after" => {
            let at = obj.get("at").and_then(serde_json::Value::as_i64).ok_or("missing at")?;
            let field = if field == "before" { DateField::Before } else { DateField::After };
            Ok(QueryAst::Date { field, at })
        }
        "older_than" | "newer_than" => {
            let days = obj.get("days").and_then(serde_json::Value::as_u64).ok_or("missing days")?;
            let field = if field == "older_than" { AgeField::OlderThan } else { AgeField::NewerThan };
            Ok(QueryAst::Age { field, days: u32::try_from(days).map_err(|_| "days out of range")? })
        }
        other => {
            let field: Field =
                serde_json::from_value(serde_json::Value::String(other.to_owned())).map_err(|e| e.to_string())?;
            let value = obj.get("value").and_then(serde_json::Value::as_str).ok_or("missing value")?.to_owned();
            Ok(QueryAst::Field { field, value })
        }
    }
}

/// `in:` values with fixed meaning (anything else is a split name).
pub const IN_VALUES: [&str; 8] = ["inbox", "sent", "drafts", "trash", "spam", "archive", "snoozed", "anywhere"];
/// `is:` values.
pub const IS_VALUES: [&str; 6] = ["unread", "read", "starred", "important", "snoozed", "needs_reply"];
/// `has:` values.
pub const HAS_VALUES: [&str; 1] = ["attachment"];
/// `category:` values (triage categories).
pub const CATEGORY_VALUES: [&str; 9] =
    ["important", "personal", "updates", "newsletters", "promotions", "receipts", "calendar", "notifications", "other"];

/// The result of [`parse`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseResult {
    /// The query.
    pub ast: QueryAst,
    /// Problems found; the AST is still usable.
    pub errors: Vec<String>,
}

/// Maximum group nesting; deeper parentheses are reported and treated as plain separators.
pub const MAX_DEPTH: usize = 32;

/// Parses a query. `now_at` (ms) and `tz` (IANA name) resolve dates; Convex queries pass the
/// client's minute-rounded `nowAt` because they cannot read the clock.
pub fn parse(q: &str, now_at: i64, tz: &str) -> ParseResult {
    let mut p =
        Parser { chars: q.chars().collect(), pos: 0, errors: Vec::new(), now_at, tz, depth: 0, too_deep: false };
    p.skip_ws();
    let mut ast = if p.at_end() { QueryAst::empty() } else { p.or_expr() };
    loop {
        p.skip_ws();
        if p.at_end() {
            break;
        }
        // A stray ')' at top level: reported (at most 16 errors) and skipped.
        if p.peek() == Some(')') {
            if p.errors.len() < 16 {
                p.errors.push(format!("unmatched ')' at {}", p.pos));
            }
            p.pos += 1;
            continue;
        }
        // Anything after a stray ')' is parsed and AND-ed on, however many there are.
        let rest = p.or_expr();
        ast = flatten_and(vec![ast, rest]);
    }
    ParseResult { ast, errors: p.errors }
}

fn flatten_and(items: Vec<QueryAst>) -> QueryAst {
    let mut out = Vec::new();
    for i in items {
        match i {
            QueryAst::And(v) => out.extend(v),
            other => out.push(other),
        }
    }
    if out.len() == 1 { out.pop().unwrap_or_else(QueryAst::empty) } else { QueryAst::And(out) }
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    errors: Vec<String>,
    now_at: i64,
    tz: &'a str,
    depth: usize,
    too_deep: bool,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) -> bool {
        let start = self.pos;
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
        self.pos > start
    }

    /// `OR` as a standalone token at the current position (after whitespace).
    fn at_or(&self) -> bool {
        self.chars.get(self.pos) == Some(&'O')
            && self.chars.get(self.pos + 1) == Some(&'R')
            && self.chars.get(self.pos + 2).is_none_or(|c| c.is_whitespace())
    }

    fn or_expr(&mut self) -> QueryAst {
        let mut alts = vec![self.and_expr()];
        loop {
            let save = self.pos;
            self.skip_ws();
            if self.at_or() && save < self.pos {
                self.pos += 2;
                if !self.skip_ws() || self.at_end() || self.peek() == Some(')') {
                    // "a OR" at the end: OR is a plain word.
                    alts.push(QueryAst::Text { text: "OR".into(), phrase: false });
                    self.errors.push("OR needs a term on both sides".into());
                    continue;
                }
                alts.push(self.and_expr());
            } else {
                self.pos = save;
                break;
            }
        }
        if alts.len() == 1 { alts.pop().unwrap_or_else(QueryAst::empty) } else { QueryAst::Or(alts) }
    }

    fn and_expr(&mut self) -> QueryAst {
        let mut items = Vec::new();
        loop {
            let save = self.pos;
            self.skip_ws();
            if self.at_end() || self.peek() == Some(')') {
                self.pos = save;
                break;
            }
            if self.at_or() && !items.is_empty() {
                self.pos = save;
                break;
            }
            if self.at_or() && items.is_empty() {
                // Leading "OR": a plain word.
                self.pos += 2;
                self.errors.push("OR needs a term on both sides".into());
                items.push(QueryAst::Text { text: "OR".into(), phrase: false });
                continue;
            }
            let item = self.unary();
            if !item.is_empty() {
                items.push(item);
            }
        }
        if items.len() == 1 { items.pop().unwrap_or_else(QueryAst::empty) } else { QueryAst::And(items) }
    }

    fn unary(&mut self) -> QueryAst {
        if self.peek() == Some('-') {
            let next = self.chars.get(self.pos + 1).copied();
            if next.is_some_and(|c| !c.is_whitespace() && c != ')') {
                self.pos += 1;
                return QueryAst::Not(Box::new(self.atom()));
            }
        }
        self.atom()
    }

    fn atom(&mut self) -> QueryAst {
        match self.peek() {
            Some('(') => {
                let open = self.pos;
                self.pos += 1;
                if self.depth >= MAX_DEPTH {
                    // Too deep: the '(' is dropped (its ')' is reported as unmatched later), so
                    // hostile input cannot grow the stack.
                    if !self.too_deep {
                        self.too_deep = true;
                        self.errors.push(format!("groups nested too deeply at {open}"));
                    }
                    return QueryAst::empty();
                }
                self.depth += 1;
                let inner = self.or_expr_in_group();
                self.depth -= 1;
                inner
            }
            Some('"') => {
                let text = self.phrase();
                QueryAst::Text { text, phrase: true }
            }
            _ => self.word_or_field(),
        }
    }

    fn or_expr_in_group(&mut self) -> QueryAst {
        let open = self.pos.saturating_sub(1);
        self.skip_ws();
        let inner = if self.peek() == Some(')') { QueryAst::empty() } else { self.or_expr() };
        self.skip_ws();
        if self.peek() == Some(')') {
            self.pos += 1;
        } else {
            self.errors.push(format!("unmatched '(' at {open}"));
        }
        inner
    }

    fn phrase(&mut self) -> String {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '"' => return out,
                '\\' => match self.peek() {
                    Some(n @ ('"' | '\\')) => {
                        out.push(n);
                        self.pos += 1;
                    }
                    _ => out.push('\\'),
                },
                c => out.push(c),
            }
        }
        self.errors.push(format!("unterminated quote at {start}"));
        out
    }

    fn word(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '"' || c == '(' || c == ')' {
                break;
            }
            out.push(c);
            self.pos += 1;
        }
        out
    }

    fn word_or_field(&mut self) -> QueryAst {
        let start = self.pos;
        // Read the field name candidate up to ':' (only word characters can form a field).
        let mut name = String::new();
        let mut i = self.pos;
        while let Some(&c) = self.chars.get(i) {
            if c == ':' || c.is_whitespace() || c == '"' || c == '(' || c == ')' {
                break;
            }
            name.push(c);
            i += 1;
        }
        if self.chars.get(i) == Some(&':')
            && let Some(kind) = field_kind(&name.to_ascii_lowercase())
        {
            self.pos = i + 1;
            let (value, quoted) = match self.peek() {
                Some('"') => (self.phrase(), true),
                _ => (self.word(), false),
            };
            let raw: String = self.chars[start..self.pos].iter().collect();
            if value.is_empty() && !quoted {
                self.errors.push(format!("missing value for {}:", name.to_ascii_lowercase()));
                return QueryAst::Text { text: raw, phrase: false };
            }
            return self.field(kind, &value, raw);
        }
        let word = self.word();
        if word.is_empty() {
            // Defensive: never loop on an unexpected character.
            let c = self.peek().map(String::from).unwrap_or_default();
            self.pos += 1;
            return QueryAst::Text { text: c, phrase: false };
        }
        QueryAst::Text { text: word, phrase: false }
    }

    fn field(&mut self, kind: FieldKind, value: &str, raw: String) -> QueryAst {
        match kind {
            FieldKind::Str(field) => {
                let value = match field {
                    Field::Is | Field::Has | Field::Category => value.to_lowercase(),
                    Field::In if IN_VALUES.contains(&value.to_lowercase().as_str()) => value.to_lowercase(),
                    _ => value.to_owned(),
                };
                let known: Option<&[&str]> = match field {
                    Field::Is => Some(&IS_VALUES),
                    Field::Has => Some(&HAS_VALUES),
                    Field::Category => Some(&CATEGORY_VALUES),
                    _ => None,
                };
                if let Some(known) = known
                    && !known.contains(&value.as_str())
                {
                    self.errors.push(format!("unknown value for {}: {value}", field.as_str()));
                }
                QueryAst::Field { field, value }
            }
            FieldKind::Date(field) => match crate::date::start_of_day_ms(value, self.tz) {
                Ok(at) => QueryAst::Date { field, at },
                Err(e) => {
                    self.errors.push(e);
                    QueryAst::Text { text: raw, phrase: false }
                }
            },
            FieldKind::Age(field) => match crate::date::relative_days(value) {
                Some(days) => {
                    let _ = self.now_at; // resolved by the backend against nowAt
                    QueryAst::Age { field, days }
                }
                None => {
                    self.errors.push(format!("invalid relative date: {value}"));
                    QueryAst::Text { text: raw, phrase: false }
                }
            },
        }
    }
}

#[derive(Clone, Copy)]
enum FieldKind {
    Str(Field),
    Date(DateField),
    Age(AgeField),
}

fn field_kind(name: &str) -> Option<FieldKind> {
    Some(match name {
        "from" => FieldKind::Str(Field::From),
        "to" => FieldKind::Str(Field::To),
        "cc" => FieldKind::Str(Field::Cc),
        "subject" => FieldKind::Str(Field::Subject),
        "label" => FieldKind::Str(Field::Label),
        "in" => FieldKind::Str(Field::In),
        "is" => FieldKind::Str(Field::Is),
        "has" => FieldKind::Str(Field::Has),
        "list" => FieldKind::Str(Field::List),
        "filename" => FieldKind::Str(Field::Filename),
        "account" => FieldKind::Str(Field::Account),
        "category" => FieldKind::Str(Field::Category),
        "before" => FieldKind::Date(DateField::Before),
        "after" => FieldKind::Date(DateField::After),
        "older_than" => FieldKind::Age(AgeField::OlderThan),
        "newer_than" => FieldKind::Age(AgeField::NewerThan),
        _ => return None,
    })
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty() || s.chars().any(|c| c.is_whitespace() || c == '"' || c == '(' || c == ')') || s == "OR"
}

fn value_str(s: &str) -> String {
    if needs_quotes(s) { quote(s) } else { s.to_owned() }
}

/// Renders an AST back to canonical query text (`normalized` in the fixtures). Dates are written as
/// `YYYY-MM-DD` in `tz`.
pub fn normalize(ast: &QueryAst, tz: &str) -> String {
    match ast {
        QueryAst::And(items) => items
            .iter()
            .map(|i| match i {
                QueryAst::Or(_) => format!("({})", normalize(i, tz)),
                _ => normalize(i, tz),
            })
            .collect::<Vec<_>>()
            .join(" "),
        QueryAst::Or(items) => items
            .iter()
            .map(|i| match i {
                QueryAst::And(v) if v.len() != 1 => format!("({})", normalize(i, tz)),
                _ => normalize(i, tz),
            })
            .collect::<Vec<_>>()
            .join(" OR "),
        QueryAst::Not(inner) => match inner.as_ref() {
            QueryAst::And(_) | QueryAst::Or(_) => format!("-({})", normalize(inner, tz)),
            _ => format!("-{}", normalize(inner, tz)),
        },
        QueryAst::Text { text, phrase } => {
            if *phrase || needs_quotes(text) || text.starts_with('-') {
                quote(text)
            } else {
                text.clone()
            }
        }
        QueryAst::Field { field, value } => format!("{}:{}", field.as_str(), value_str(value)),
        QueryAst::Date { field, at } => {
            let name = match field {
                DateField::Before => "before",
                DateField::After => "after",
            };
            format!("{name}:{}", crate::date::format_day(*at, tz))
        }
        QueryAst::Age { field, days } => {
            let name = match field {
                AgeField::OlderThan => "older_than",
                AgeField::NewerThan => "newer_than",
            };
            format!("{name}:{days}d")
        }
    }
}

impl fmt::Display for QueryAst {
    /// Normalized text with dates in UTC.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&normalize(self, "UTC"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_792_454_400_000; // 2026-10-19T00:00:00Z
    const TZ: &str = "Europe/Zurich";

    fn p(q: &str) -> ParseResult {
        parse(q, NOW, TZ)
    }

    fn ast(q: &str) -> serde_json::Value {
        serde_json::to_value(p(q).ast).unwrap()
    }

    #[test]
    fn empty_and_words() {
        assert_eq!(ast(""), json!({"and": []}));
        assert_eq!(ast("   "), json!({"and": []}));
        assert_eq!(ast("hello"), json!({"text": "hello", "phrase": false}));
        assert_eq!(
            ast(" hello  world "),
            json!({"and": [{"text": "hello", "phrase": false}, {"text": "world", "phrase": false}]})
        );
    }

    #[test]
    fn fields_are_case_insensitive() {
        assert_eq!(ast("FROM:Ada@Example.com"), json!({"field": "from", "value": "Ada@Example.com"}));
        assert_eq!(ast("is:UNREAD"), json!({"field": "is", "value": "unread"}));
        assert_eq!(ast("has:attachment"), json!({"field": "has", "value": "attachment"}));
        assert_eq!(ast("label:\"Work Stuff\""), json!({"field": "label", "value": "Work Stuff"}));
        assert_eq!(ast("in:Inbox"), json!({"field": "in", "value": "inbox"}));
        assert_eq!(ast("in:Newsletters"), json!({"field": "in", "value": "Newsletters"}));
    }

    #[test]
    fn unknown_field_is_a_word() {
        assert_eq!(ast("x:y"), json!({"text": "x:y", "phrase": false}));
        assert_eq!(ast("https://a.b/c"), json!({"text": "https://a.b/c", "phrase": false}));
    }

    #[test]
    fn phrases_and_escapes() {
        assert_eq!(ast(r#""quarterly report""#), json!({"text": "quarterly report", "phrase": true}));
        assert_eq!(ast(r#""say \"hi\" \\ bye""#), json!({"text": "say \"hi\" \\ bye", "phrase": true}));
        let r = p(r#""open"#);
        assert_eq!(r.ast, QueryAst::Text { text: "open".into(), phrase: true });
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn negation_and_or_and_groups() {
        assert_eq!(ast("-spam"), json!({"not": {"text": "spam", "phrase": false}}));
        assert_eq!(ast("-from:a@b.c"), json!({"not": {"field": "from", "value": "a@b.c"}}));
        assert_eq!(
            ast("a OR b c"),
            json!({"or": [{"text": "a", "phrase": false}, {"and": [{"text": "b", "phrase": false}, {"text": "c", "phrase": false}]}]})
        );
        assert_eq!(
            ast("(from:a OR from:b) is:unread"),
            json!({"and": [{"or": [{"field": "from", "value": "a"}, {"field": "from", "value": "b"}]}, {"field": "is", "value": "unread"}]})
        );
        assert_eq!(
            ast("-(a b)"),
            json!({"not": {"and": [{"text": "a", "phrase": false}, {"text": "b", "phrase": false}]}})
        );
        // Lowercase "or" is a word.
        assert_eq!(ast("a or b")["and"].as_array().unwrap().len(), 3);
        // A lone dash is a word.
        assert_eq!(ast("a - b")["and"][1], json!({"text": "-", "phrase": false}));
    }

    #[test]
    fn dates_resolve_in_timezone() {
        // 2026-10-01 00:00 in Zurich is 2026-09-30T22:00:00Z.
        assert_eq!(ast("after:2026-10-01"), json!({"field": "after", "at": 1_790_805_600_000_i64}));
        assert_eq!(ast("before:2026/10/01"), json!({"field": "before", "at": 1_790_805_600_000_i64}));
        assert_eq!(ast("older_than:2w"), json!({"field": "older_than", "days": 14}));
        assert_eq!(ast("newer_than:3d"), json!({"field": "newer_than", "days": 3}));
        assert_eq!(ast("newer_than:1y"), json!({"field": "newer_than", "days": 365}));
        let r = p("before:2026-13-01");
        assert_eq!(r.ast, QueryAst::Text { text: "before:2026-13-01".into(), phrase: false });
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn errors_are_reported_but_parsing_continues() {
        let r = p("(a b");
        assert_eq!(r.errors.len(), 1);
        assert_eq!(serde_json::to_value(&r.ast).unwrap()["and"].as_array().unwrap().len(), 2);
        let r = p("a) b) c");
        assert_eq!(r.errors.len(), 2);
        assert_eq!(serde_json::to_value(&r.ast).unwrap()["and"].as_array().unwrap().len(), 3, "nothing is dropped");
        let r = p("a) b");
        assert_eq!(r.errors.len(), 1);
        assert_eq!(
            r.ast,
            QueryAst::And(vec![
                QueryAst::Text { text: "a".into(), phrase: false },
                QueryAst::Text { text: "b".into(), phrase: false }
            ])
        );
        assert!(!p("is:bogus").errors.is_empty());
        let r = p("from:");
        assert_eq!(r.ast, QueryAst::Text { text: "from:".into(), phrase: false });
        assert_eq!(r.errors.len(), 1);
    }

    #[test]
    fn deep_nesting_does_not_overflow() {
        let q = "(".repeat(10_000) + "x" + &")".repeat(10_000);
        let r = p(&q);
        assert!(!r.errors.is_empty());
    }

    #[test]
    fn normalize_round_trips() {
        for q in [
            "hello",
            "from:a@b.c is:unread",
            "\"two words\" -spam",
            "(a OR b) c",
            "a OR b",
            "label:\"Work Stuff\" has:attachment",
            "after:2026-10-01 before:2026-10-20",
            "older_than:14d",
            "-(a b)",
        ] {
            let parsed = p(q);
            assert!(parsed.errors.is_empty(), "{q}: {:?}", parsed.errors);
            let n = normalize(&parsed.ast, TZ);
            assert_eq!(n, q, "normalize({q})");
            assert_eq!(p(&n).ast, parsed.ast);
        }
    }

    #[test]
    fn serde_round_trip() {
        for q in ["a OR -b", "after:2026-01-01 from:x", "older_than:1m", "\"p q\""] {
            let a = p(q).ast;
            let back: QueryAst = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
            assert_eq!(back, a);
        }
    }

    #[test]
    fn shared_fixtures() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/mail-model/fixtures/query.json");
        let text = std::fs::read_to_string(&path).expect("packages/mail-model/fixtures/query.json is checked in");
        let n = crate::fixtures::run_query_fixtures(&text).unwrap_or_else(|e| panic!("{e}"));
        assert!(n >= 150, "only {n} query cases");
    }
}
