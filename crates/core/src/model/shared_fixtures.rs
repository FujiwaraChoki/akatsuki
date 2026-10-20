//! Runs the shared mail-model fixtures (`packages/mail-model/fixtures/*.json`, ARCHITECTURE §1.7).
//! The TypeScript side (`convex/lib/model.ts` until `packages/mail-model` exists, D-126) runs the same
//! files in `convex/test/fixtures.test.ts`.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::Addr;
use super::apply_ops::{ApplyContext, Effect, Visible, apply_ops};
use super::label_key::{LabelKey, SplitKey, parse_label_key};
use super::rules::{RuleSubject, match_rule};
use crate::convex::{Rule, ThreadAction, TriageCategory};

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/mail-model/fixtures").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn cases(fx: &Value) -> &Vec<Value> {
    fx["cases"].as_array().expect("cases")
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|s| s.as_str().unwrap().to_owned()).collect()).unwrap_or_default()
}

/// The fixture's JSON shape of a parsed key (`ParsedLabelKey` in TypeScript).
fn parsed_json(k: &LabelKey) -> Value {
    match k {
        LabelKey::Sys(role) => json!({"kind": "sys", "role": role.as_str()}),
        LabelKey::User(id) => json!({"kind": "user", "labelId": id}),
        LabelKey::Split(SplitKey::Other) => json!({"kind": "split", "splitId": "other"}),
        LabelKey::Split(SplitKey::Split(id)) => json!({"kind": "split", "splitId": id}),
    }
}

#[test]
fn label_keys() {
    let fx = fixture("label-keys.json");
    for case in cases(&fx) {
        let input = case["input"].as_str().unwrap();
        let got = parse_label_key(input).map(|k| parsed_json(&k)).unwrap_or(Value::Null);
        assert_eq!(got, case["parsed"], "{input:?}");
        // Display round-trips every valid key.
        if let Ok(k) = parse_label_key(input) {
            assert_eq!(k.to_string(), input);
        }
    }
    assert!(cases(&fx).len() >= 30);
}

#[test]
fn apply_ops_cases() {
    let fx = fixture("apply-ops.json");
    for case in cases(&fx) {
        let name = case["name"].as_str().unwrap();
        let ctx = match case["provider"].as_str().unwrap() {
            "gmail" => ApplyContext::gmail(),
            _ => ApplyContext::folders(strings(&case["exclusiveLabelIds"])),
        };
        let actions: Vec<ThreadAction> = serde_json::from_value(case["actions"].clone()).unwrap();
        let synced = strings(&case["synced"]);
        let visible = apply_ops(&synced, &actions, &ctx);
        let expected: Visible = serde_json::from_value(case["expected"].clone()).unwrap();
        assert_eq!(visible, expected, "{name}");
        let start = apply_ops(&synced, [], &ctx);
        let effect: Effect = serde_json::from_value(case["effect"].clone()).unwrap();
        assert_eq!(Effect::between(&start, &visible), effect, "{name} effect");
    }
    assert!(cases(&fx).len() >= 20);
}

#[test]
fn rules_cases() {
    let fx = fixture("rules.json");
    for case in cases(&fx) {
        let name = case["name"].as_str().unwrap();
        let rule: Rule = serde_json::from_value(case["rule"].clone()).unwrap();
        let f = &case["facts"];
        let from = f["fromEmail"].as_str().map(|e| Addr { name: None, email: e.to_owned() });
        let participants: Vec<Addr> =
            strings(&f["participants"]).into_iter().map(|email| Addr { name: None, email }).collect();
        let label_keys = strings(&f["labelKeys"]);
        let triage_category: Option<TriageCategory> =
            f.get("triage").filter(|t| !t.is_null()).map(|t| serde_json::from_value(t["category"].clone()).unwrap());
        let needs_reply = f.get("triage").and_then(|t| t["needsReply"].as_bool());
        let vip = f["vip"].as_bool().unwrap();
        let subject = RuleSubject {
            last_from: from.as_ref(),
            participants: &participants,
            label_keys: &label_keys,
            triage_category,
            needs_reply,
            is_bulk: f["isBulk"].as_bool().unwrap(),
            list_id: f["listId"].as_str(),
            subject: f["subject"].as_str().unwrap(),
            account_id: f["accountId"].as_str().unwrap(),
        };
        let got = match_rule(&rule, &subject, &|_| vip);
        assert_eq!(got, case["expected"].as_bool().unwrap(), "{name}");
    }
    assert!(cases(&fx).len() >= 30);
}
