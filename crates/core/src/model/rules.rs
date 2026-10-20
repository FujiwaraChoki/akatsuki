//! Split and filter rule matching (ARCHITECTURE §3.4, `vRule` in §3.1).
//!
//! Rules see metadata only: `lastFrom.email`, `participants`, `labelKeys`, `triage`, `isBulk`,
//! `listId`, `subject`, `accountId` and VIP status. Rust twin of `matchRule` in
//! `packages/mail-model` (interim: `convex/lib/model.ts`, D-126); both run
//! `packages/mail-model/fixtures/rules.json`.

use crate::convex::{Rule, RuleCond, RuleMatch, TriageCategory};
use crate::model::Addr;

/// The thread fields a rule may look at.
#[derive(Clone, Debug, Default)]
pub struct RuleSubject<'a> {
    /// `threads.lastFrom`.
    pub last_from: Option<&'a Addr>,
    /// `threads.participants`.
    pub participants: &'a [Addr],
    /// `threads.labelKeys` (visible keys).
    pub label_keys: &'a [String],
    /// Triage category, if triage has run.
    pub triage_category: Option<TriageCategory>,
    /// Triage `needsReply`, if triage has run.
    pub needs_reply: Option<bool>,
    /// `threads.isBulk`.
    pub is_bulk: bool,
    /// `threads.listId`.
    pub list_id: Option<&'a str>,
    /// `threads.subject`.
    pub subject: &'a str,
    /// `threads.accountId`.
    pub account_id: &'a str,
}

fn domain_matches(email: &str, domain: &str) -> bool {
    let lower = domain.to_lowercase();
    let domain = lower.strip_prefix('@').unwrap_or(&lower);
    match email.rsplit_once('@') {
        Some((_, d)) => {
            let d = d.to_lowercase();
            d == domain || d.ends_with(&format!(".{domain}"))
        }
        None => false,
    }
}

/// Evaluates one condition. `is_vip` answers whether a sender address is a VIP
/// (`senderPrefs.vip` for the address or its `@domain`).
pub fn match_cond(cond: &RuleCond, s: &RuleSubject<'_>, is_vip: &dyn Fn(&str) -> bool) -> bool {
    let from = s.last_from.map(|a| a.email.to_lowercase());
    match cond {
        RuleCond::FromEmail { eq } => from.as_deref() == Some(eq.to_lowercase().as_str()),
        RuleCond::FromDomain { eq } => from.as_deref().is_some_and(|f| domain_matches(f, eq)),
        RuleCond::ToEmail { eq } => {
            let eq = eq.to_lowercase();
            s.participants.iter().any(|p| p.email.to_lowercase() == eq)
        }
        RuleCond::Label { key } => s.label_keys.iter().any(|k| k == key),
        RuleCond::Triage { category } => s.triage_category == Some(*category),
        RuleCond::NeedsReply { eq } => s.needs_reply == Some(*eq),
        RuleCond::IsBulk { eq } => s.is_bulk == *eq,
        RuleCond::ListId { eq } => s.list_id.is_some_and(|l| l.to_lowercase() == eq.to_lowercase()),
        RuleCond::SubjectContains { text } => {
            !text.is_empty() && s.subject.to_lowercase().contains(&text.to_lowercase())
        }
        RuleCond::Vip { eq } => from.as_deref().is_some_and(is_vip) == *eq,
        RuleCond::Account { account_id } => s.account_id == account_id.as_str(),
    }
}

/// Evaluates a rule: `all` needs every condition, `any` needs at least one. A rule with no
/// conditions never matches (validators require 1..20).
pub fn match_rule(rule: &Rule, s: &RuleSubject<'_>, is_vip: &dyn Fn(&str) -> bool) -> bool {
    if rule.conds.is_empty() {
        return false;
    }
    match rule.r#match {
        RuleMatch::All => rule.conds.iter().all(|c| match_cond(c, s, is_vip)),
        RuleMatch::Any => rule.conds.iter().any(|c| match_cond(c, s, is_vip)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject<'a>(from: &'a Addr, labels: &'a [String]) -> RuleSubject<'a> {
        RuleSubject {
            last_from: Some(from),
            participants: std::slice::from_ref(from),
            label_keys: labels,
            subject: "Your Invoice #42",
            account_id: "acc1",
            ..Default::default()
        }
    }

    #[test]
    fn conditions() {
        let from = Addr::new(Some("Billing"), "billing@mail.Stripe.com");
        let labels = vec!["sys:inbox".to_owned()];
        let s = subject(&from, &labels);
        let no_vip = |_: &str| false;
        assert!(match_cond(&RuleCond::FromDomain { eq: "stripe.com".into() }, &s, &no_vip));
        assert!(!match_cond(&RuleCond::FromDomain { eq: "tripe.com".into() }, &s, &no_vip));
        assert!(match_cond(&RuleCond::FromEmail { eq: "Billing@mail.stripe.com".into() }, &s, &no_vip));
        assert!(!match_cond(&RuleCond::SubjectContains { text: String::new() }, &s, &no_vip));
        assert!(match_cond(&RuleCond::SubjectContains { text: "invoice".into() }, &s, &no_vip));
        assert!(match_cond(&RuleCond::Label { key: "sys:inbox".into() }, &s, &no_vip));
        assert!(!match_cond(&RuleCond::NeedsReply { eq: false }, &s, &no_vip), "no triage yet");
        assert!(match_cond(&RuleCond::Vip { eq: false }, &s, &no_vip));
        assert!(match_cond(&RuleCond::Account { account_id: "acc1".into() }, &s, &no_vip));
    }

    #[test]
    fn all_and_any() {
        let from = Addr::new(None, "a@b.com");
        let labels = vec![];
        let s = subject(&from, &labels);
        let yes = RuleCond::FromDomain { eq: "b.com".into() };
        let no = RuleCond::IsBulk { eq: true };
        let all = Rule { r#match: RuleMatch::All, conds: vec![yes.clone(), no.clone()] };
        let any = Rule { r#match: RuleMatch::Any, conds: vec![yes, no] };
        assert!(!match_rule(&all, &s, &|_| false));
        assert!(match_rule(&any, &s, &|_| false));
        assert!(!match_rule(&Rule { r#match: RuleMatch::All, conds: vec![] }, &s, &|_| false));
    }
}
