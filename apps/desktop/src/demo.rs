//! The demo mailbox (D-316): synthetic accounts, labels, splits and threads, served by an
//! in-process stand-in for Convex and the sync worker. It implements the same
//! [`ClientApi`]/[`MailServer`] traits as production, so the mirror feed, the overlay, optimistic
//! actions, undo, the body renderer, search and compose all run their real code paths with no
//! server, no account and no network. Every name, address and text is invented; the domains are
//! reserved `.example` names. The UI labels it "Demo mailbox".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use akatsuki_convex_api::{ClientApi, ConvexApiError, Subscription};
use akatsuki_core::convex::client::{
    AckFailureArgs, CancelOutcome, ClaimTriageArgs, MailApplyArgs, MailApplyResult, MailSetReminderArgs,
    MailSetReminderResult, MailSnoozeArgs, MailSnoozeResult, MailUndoArgs, MailUndoResult, RegisterDeviceArgs,
    SaveAiSettingsArgs, SendsCancelArgs, SendsCancelResult, SetTriageArgs, SplitView, SyncChangesArgs,
    SyncChangesResult, TombstoneRef, UndoMode,
};
use akatsuki_core::convex::rows::{
    AiSettings, LabelView, MailAccountView, MessageMeta, OpFailure, ThreadMirror, TriageInput,
};
use akatsuki_core::convex::{ThreadAction, TombstoneTable};
use akatsuki_core::model::apply_ops::Effect;
use akatsuki_core::model::{ApplyContext, Visible, apply_action};
#[cfg(test)]
use akatsuki_core::store::{Store, StoreError};
use akatsuki_core::time::{DAY_MS, MINUTE_MS, now_ms};
use akatsuki_core::wire::worker::{DraftPut, DraftSaved, SendAccepted, SendRequest};

use crate::remote::MailServer;
use crate::worker::WorkerError;

/// The demo Gmail account.
pub const GMAIL: &str = "demo-gmail";
/// The demo Outlook account.
pub const OUTLOOK: &str = "demo-outlook";
/// The demo user's address.
pub const ME: &str = "me@akatsuki-demo.example";
const ME_OUTLOOK: &str = "me@outlook-demo.example";

fn account(id: &str, provider: &str, email: &str, order: i64) -> Value {
    json!({
        "_id": id, "_creationTime": 1.0, "provider": provider, "providerAccountId": email, "email": email,
        "displayName": if provider == "gmail" { "Personal" } else { "Work" },
        "sortOrder": order, "status": "live", "syncPhase": "complete", "progress": {"threadsDone": 0},
        "windowStartAt": 0, "backfillHorizonAt": 0,
        "caps": {"multiLabel": provider == "gmail", "categories": provider == "graph", "serverSearch": true,
                 "deleteForever": provider != "gmail"},
        "createdAt": 1
    })
}

fn label(id: &str, account: &str, name: &str, exclusive: bool, role: Option<&str>) -> Value {
    let mut v = json!({
        "_id": id, "_creationTime": 1.0, "accountId": account, "providerLabelId": format!("p-{id}"), "name": name,
        "kind": if exclusive { "folder" } else { "user" }, "exclusive": exclusive, "hidden": false,
        "pendingDelete": false, "labelKey": format!("l:{id}")
    });
    if let Some(r) = role {
        v["role"] = json!(r);
    }
    v
}

/// The demo's split tabs.
pub fn splits() -> Vec<SplitView> {
    serde_json::from_value(json!([
        {"_id": "vip", "_creationTime": 1.0, "name": "VIP", "order": 0, "showCount": true, "labelKey": "split:vip",
         "rule": {"match": "any", "conds": [{"f": "vip", "eq": true}]}},
        {"_id": "team", "_creationTime": 1.0, "name": "Team", "order": 1, "showCount": true, "labelKey": "split:team",
         "rule": {"match": "any", "conds": [{"f": "from_domain", "eq": "northwind.example"}]}}
    ]))
    .unwrap_or_default()
}

struct Msg {
    from: (&'static str, &'static str),
    ago: i64,
    text: String,
    html: Option<String>,
    unread: bool,
}

struct Spec {
    id: String,
    account: &'static str,
    subject: String,
    keys: Vec<&'static str>,
    split: Option<&'static str>,
    messages: Vec<Msg>,
    extra: Value,
}

fn msg(from: (&'static str, &'static str), ago: i64, text: &str) -> Msg {
    Msg { from, ago, text: text.to_owned(), html: None, unread: false }
}

const MIRA: (&str, &str) = ("Mira Okafor", "mira@northwind.example");
const JONAS: (&str, &str) = ("Jonas Weber", "jonas@northwind.example");
const PRIYA: (&str, &str) = ("Priya Raman", "priya@northwind.example");
const TOMAS: (&str, &str) = ("Tomás Ruiz", "tomas@northwind.example");
const HANA: (&str, &str) = ("Hana Ito", "hana@kitsune-mail.example");
const STUDIO: (&str, &str) = ("Studio Mori", "letters@studio-mori.example");
const KETTLE: (&str, &str) = ("Kettle & Co", "receipts@kettle.example");
const AIR: (&str, &str) = ("Cloudline Air", "bookings@cloudline.example");
const ME_ADDR: (&str, &str) = ("Me", ME);
const LOTTERY: (&str, &str) = ("Prize Desk", "winner@prize-desk.example");
const ROBIN: (&str, &str) = ("Robin Achterberg", "robin@contoso-demo.example");
const FINANCE: (&str, &str) = ("Finance Team", "finance@contoso-demo.example");

fn newsletter_html(issue: u32) -> String {
    format!(
        r#"<html><head><style>.cta{{color:#C1272D}} body{{font-family:Georgia}}</style></head><body>
<div style="display:none">Preview text: this week in slow design</div>
<table width="600" cellpadding="0" cellspacing="0"><tr><td>
<h1>The dawn issue, no. {issue}</h1>
<p>Good morning. This week we look at <b>quiet interfaces</b>, <i>morning light</i> and one very good teapot.</p>
<img src="https://cdn.studio-mori.example/hero.jpg" alt="A teapot at sunrise" width="560">
<h2>Three things we liked</h2>
<ol><li>A calendar that only shows <a href="https://studio-mori.example/one">one day</a>.</li>
<li>Paper notebooks with <code>dot-grid</code> pages.</li>
<li>A playlist for writing letters.</li></ol>
<table><tr><th>Event</th><th>When</th></tr><tr><td>Open studio</td><td>Saturday 10:00</td></tr>
<tr><td>Letterpress class</td><td>Sunday 14:00</td></tr></table>
<blockquote>Make it slow, make it kind.</blockquote>
<p class="cta"><a href="https://studio-mori.example/read">Read the full letter</a></p>
<img src="https://track.studio-mori.example/open.gif" width="1" height="1">
<script>alert('never runs')</script>
</td></tr></table></body></html>"#
    )
}

fn specs(now: i64) -> Vec<Spec> {
    let mut v = vec![
        Spec {
            id: "t-board".into(),
            account: GMAIL,
            subject: "Board deck for Thursday".into(),
            keys: vec!["sys:inbox", "sys:unread", "sys:starred", "sys:important"],
            split: Some("split:vip"),
            messages: vec![
                msg(
                    MIRA,
                    180,
                    "Could you send me the latest board deck? I'd like to read it tonight.\n\nThanks,\nMira",
                ),
                Msg {
                    unread: true,
                    ..msg(
                        MIRA,
                        25,
                        "One more thing: please add the hiring slide we discussed.\n\nOn Tuesday, Me wrote:\n> Sure, I'll send it by tonight.\n> The numbers are final.",
                    )
                },
            ],
            extra: json!({}),
        },
        Spec {
            id: "t-hiring".into(),
            account: GMAIL,
            subject: "Re: Hiring plan Q4".into(),
            keys: vec!["sys:inbox"],
            split: Some("split:vip"),
            messages: vec![
                msg(JONAS, 60 * 26, "Here is the first draft of the Q4 hiring plan. Two engineers, one designer."),
                msg(ME_ADDR, 60 * 25, "Looks good. Can we move the designer to October?"),
                msg(JONAS, 60 * 3, "Done. October it is. I also added a budget column."),
            ],
            extra: json!({}),
        },
        Spec {
            id: "t-contract".into(),
            account: GMAIL,
            subject: "Contract draft v3".into(),
            keys: vec!["sys:inbox"],
            split: Some("split:vip"),
            messages: vec![msg(MIRA, 60 * 30, "Attached is v3 of the contract. Legal signed off on sections 1 to 4.")],
            extra: json!({"reminderAt": now + DAY_MS, "hasAttachments": true}),
        },
        Spec {
            id: "t-review".into(),
            account: GMAIL,
            subject: "Design review notes".into(),
            keys: vec!["sys:inbox", "sys:unread"],
            split: Some("split:team"),
            messages: vec![Msg {
                html: Some(
                    "<p>Notes from today's review:</p><ul><li>Keep the crimson accent single.</li>\
                     <li>Move the split tabs under the search bar.</li><li>Ship density settings.</li></ul>\
                     <p>Full notes are in the <a href=\"https://wiki.northwind.example/review\">wiki</a>.</p>"
                        .into(),
                ),
                unread: true,
                ..msg(PRIYA, 95, "Notes from today's review: keep the accent single, move split tabs, ship density.")
            }],
            extra: json!({"hasAttachments": true}),
        },
        Spec {
            id: "t-staging".into(),
            account: GMAIL,
            subject: "Staging is green again".into(),
            keys: vec!["sys:inbox"],
            split: Some("split:team"),
            messages: vec![msg(
                TOMAS,
                60 * 5,
                "The flaky test was a timezone bug. Staging has been green for two hours.",
            )],
            extra: json!({}),
        },
        Spec {
            id: "t-dawn".into(),
            account: GMAIL,
            subject: "The dawn issue, no. 42".into(),
            keys: vec!["sys:inbox", "sys:unread"],
            split: None,
            messages: vec![Msg {
                html: Some(newsletter_html(42)),
                unread: true,
                ..msg(
                    STUDIO,
                    60 * 7,
                    "Good morning. This week we look at quiet interfaces, morning light and one very good teapot.",
                )
            }],
            extra: json!({"isBulk": true, "listId": "letters.studio-mori.example",
                          "listUnsubscribe": {"http": "https://studio-mori.example/u/1", "oneClick": true}}),
        },
        Spec {
            id: "t-receipt".into(),
            account: GMAIL,
            subject: "Your receipt from Kettle & Co".into(),
            keys: vec!["sys:inbox", "l:receipts"],
            split: None,
            messages: vec![msg(
                KETTLE,
                60 * 20,
                "Thank you for your order. Total: 38.50. One cast-iron kettle, one tin of sencha.",
            )],
            extra: json!({}),
        },
        Spec {
            id: "t-flight".into(),
            account: GMAIL,
            subject: "Flight confirmation ZRH to HND".into(),
            keys: vec!["sys:inbox", "sys:unread", "sys:starred", "l:travel"],
            split: None,
            messages: vec![Msg {
                unread: true,
                ..msg(
                    AIR,
                    60 * 9,
                    "Your booking is confirmed. Departure 21:40 from Zurich, seat 32A. Check-in opens 24 hours before.",
                )
            }],
            extra: json!({"hasAttachments": true}),
        },
        Spec {
            id: "t-coffee".into(),
            account: GMAIL,
            subject: "Coffee next week?".into(),
            keys: vec!["sys:inbox", "sys:sent"],
            split: None,
            messages: vec![
                msg(HANA, 60 * 50, "I'm in town next week. Coffee on Wednesday?"),
                msg(ME_ADDR, 60 * 49, "Wednesday works. 9:00 at the usual place?"),
            ],
            extra: json!({}),
        },
        Spec {
            id: "t-passport".into(),
            account: GMAIL,
            subject: "Renew passport".into(),
            keys: vec!["sys:snoozed"],
            split: None,
            messages: vec![msg(ME_ADDR, 60 * 72, "Reminder to self: book the passport appointment.")],
            extra: json!({"snoozedUntil": now + 2 * DAY_MS}),
        },
        Spec {
            id: "t-invoice-old".into(),
            account: GMAIL,
            subject: "Invoice for September".into(),
            keys: vec!["l:receipts"],
            split: None,
            messages: vec![msg(KETTLE, 60 * 24 * 20, "Your September invoice is ready.")],
            extra: json!({}),
        },
        Spec {
            id: "t-offsite".into(),
            account: GMAIL,
            subject: "Notes for the offsite".into(),
            keys: vec!["sys:drafts"],
            split: None,
            messages: vec![msg(ME_ADDR, 60 * 4, "Agenda: roadmap, hiring, the dawn release.")],
            extra: json!({"draftCount": 1}),
        },
        Spec {
            id: "t-spam".into(),
            account: GMAIL,
            subject: "You have won a prize".into(),
            keys: vec!["sys:spam"],
            split: None,
            messages: vec![msg(LOTTERY, 60 * 12, "Claim your prize now.")],
            extra: json!({}),
        },
        Spec {
            id: "t-quarter".into(),
            account: OUTLOOK,
            subject: "Quarterly numbers".into(),
            keys: vec!["sys:inbox", "sys:unread"],
            split: None,
            messages: vec![Msg {
                unread: true,
                ..msg(FINANCE, 40, "Revenue is up 12 % on last quarter. The full report is attached.")
            }],
            extra: json!({"hasAttachments": true}),
        },
        Spec {
            id: "t-lunch".into(),
            account: OUTLOOK,
            subject: "Team lunch on Friday".into(),
            keys: vec!["sys:inbox"],
            split: None,
            messages: vec![msg(ROBIN, 60 * 6, "Lunch is booked for Friday at 12:30. Vegetarian options included.")],
            extra: json!({}),
        },
        Spec {
            id: "t-roadmap".into(),
            account: OUTLOOK,
            subject: "Roadmap review".into(),
            keys: vec!["l:o-proj"],
            split: None,
            messages: vec![msg(ROBIN, 60 * 30, "Roadmap review moved to Monday.")],
            extra: json!({}),
        },
    ];
    // A longer tail so the list has to scroll.
    for i in 0..40u32 {
        let (from, subject, text) = match i % 4 {
            0 => (STUDIO, format!("The dawn issue, no. {}", 41 - i / 4), "Last week's letter.".to_owned()),
            1 => (TOMAS, format!("Deploy report #{}", 400 - i), "All services healthy.".to_owned()),
            2 => (KETTLE, format!("Order {} shipped", 9100 + i), "Your parcel is on its way.".to_owned()),
            _ => (HANA, format!("Photos from the trip, part {}", i / 4 + 1), "A few more photos.".to_owned()),
        };
        let inbox = i < 24;
        let mut keys = if inbox { vec!["sys:inbox"] } else { vec![] };
        if i % 7 == 0 {
            keys.push("sys:unread");
        }
        let split = if inbox && from == TOMAS { Some("split:team") } else { None };
        v.push(Spec {
            id: format!("t-tail-{i}"),
            account: if i % 5 == 4 { OUTLOOK } else { GMAIL },
            subject,
            keys,
            split,
            messages: vec![msg(from, 60 * 24 * (i64::from(i) + 2), &text)],
            extra: if from == STUDIO { json!({"isBulk": true}) } else { json!({}) },
        });
    }
    v
}

fn raw_message(m: &Msg, subject: &str, id: &str) -> Vec<u8> {
    let head = format!(
        "From: {} <{}>\r\nTo: Me <{ME}>\r\nSubject: {subject}\r\nMessage-ID: <{id}@akatsuki-demo.example>\r\nMIME-Version: 1.0\r\n",
        m.from.0, m.from.1
    );
    match &m.html {
        Some(html) => format!(
            "{head}Content-Type: multipart/alternative; boundary=\"demo\"\r\n\r\n--demo\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n--demo\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--demo--\r\n",
            m.text
        )
        .into_bytes(),
        None => format!("{head}Content-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n", m.text).into_bytes(),
    }
}

struct DemoState {
    accounts: Vec<MailAccountView>,
    labels: Vec<LabelView>,
    threads: BTreeMap<String, ThreadMirror>,
    messages: BTreeMap<String, MessageMeta>,
    message_rev: HashMap<String, u64>,
    raw: HashMap<String, Vec<u8>>,
    revs: BTreeMap<String, u64>,
    tombstones: Vec<(String, u64, TombstoneTable, String)>,
    splits: HashMap<String, String>,
    effects: HashMap<String, Vec<(String, Effect)>>,
    pending_sends: HashSet<String>,
    cancelled_sends: HashSet<String>,
    sent: HashSet<String>,
    ai: Option<AiSettings>,
    seq: u64,
}

impl DemoState {
    fn new(now: i64) -> Self {
        let accounts: Vec<MailAccountView> =
            serde_json::from_value(json!([account(GMAIL, "gmail", ME, 0), account(OUTLOOK, "graph", ME_OUTLOOK, 1)]))
                .unwrap_or_default();
        let labels: Vec<LabelView> = serde_json::from_value(json!([
            label("receipts", GMAIL, "Receipts", false, None),
            label("travel", GMAIL, "Travel", false, None),
            label("work", GMAIL, "Work", false, None),
            label("o-arch", OUTLOOK, "Archive", true, Some("archive")),
            label("o-proj", OUTLOOK, "Projects", true, None),
        ]))
        .unwrap_or_default();
        let mut s = Self {
            accounts,
            labels,
            threads: BTreeMap::new(),
            messages: BTreeMap::new(),
            message_rev: HashMap::new(),
            raw: HashMap::new(),
            revs: BTreeMap::from([(GMAIL.to_owned(), 1), (OUTLOOK.to_owned(), 1)]),
            tombstones: Vec::new(),
            splits: HashMap::new(),
            effects: HashMap::new(),
            pending_sends: HashSet::new(),
            cancelled_sends: HashSet::new(),
            sent: HashSet::new(),
            ai: None,
            seq: 0,
        };
        for spec in specs(now) {
            s.insert(spec, now);
        }
        s
    }

    fn insert(&mut self, spec: Spec, now: i64) {
        let last = spec.messages.iter().map(|m| now - m.ago * MINUTE_MS).max().unwrap_or(now);
        let latest = spec.messages.iter().min_by_key(|m| m.ago);
        let from = latest.map(|m| json!({"name": m.from.0, "email": m.from.1}));
        let unread =
            spec.messages.iter().filter(|m| m.unread).count().max(usize::from(spec.keys.contains(&"sys:unread")));
        let mut participants: Vec<Value> = Vec::new();
        for m in spec.messages.iter().rev() {
            let p = json!({"name": m.from.0, "email": m.from.1});
            if !participants.contains(&p) {
                participants.push(p);
            }
        }
        let mut doc = json!({
            "_id": spec.id, "_creationTime": 1.0, "accountId": spec.account, "providerThreadId": format!("p-{}", spec.id),
            "subject": spec.subject, "snippet": latest.map(|m| m.text.lines().next().unwrap_or("").chars().take(200).collect::<String>()).unwrap_or_default(),
            "participants": participants, "lastFrom": from, "isBulk": false, "lastMessageAt": last, "sortAt": last,
            "messageCount": spec.messages.len(), "unreadCount": unread, "draftCount": 0, "hasAttachments": false,
            "syncedLabelKeys": spec.keys, "labelKeys": spec.keys, "pendingOpCount": 0, "deletedPending": false,
            "triageState": "none", "rev": 1, "updatedAt": last
        });
        if let Some(sp) = spec.split {
            doc["split"] = json!(sp);
            self.splits.insert(spec.id.clone(), sp.to_owned());
        } else if spec.keys.contains(&"sys:inbox") {
            doc["split"] = json!("split:other");
        }
        if let (Value::Object(d), Value::Object(e)) = (&mut doc, spec.extra) {
            d.extend(e);
        }
        let Ok(thread) = serde_json::from_value::<ThreadMirror>(doc) else { return };
        for (i, m) in spec.messages.iter().enumerate() {
            let mid = format!("{}-m{i}", spec.id);
            let mut keys: Vec<&str> = spec.keys.iter().copied().filter(|k| *k != "sys:unread").collect();
            if m.unread {
                keys.push("sys:unread");
            }
            if m.from.1 == ME {
                keys.push("sys:sent");
            }
            let meta = json!({
                "_id": mid, "_creationTime": 1.0, "accountId": spec.account, "threadId": spec.id,
                "providerMessageId": format!("p-{mid}"), "references": [], "from": {"name": m.from.0, "email": m.from.1},
                "to": [{"name": "Me", "email": if spec.account == GMAIL { ME } else { ME_OUTLOOK }}], "cc": [], "bcc": [], "replyTo": [],
                "subject": spec.subject, "snippet": m.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>(),
                "date": now - m.ago * MINUTE_MS, "labelKeys": keys, "isUnread": m.unread, "isDraft": spec.keys.contains(&"sys:drafts"),
                "isOutbound": m.from.1 == ME, "hasAttachments": thread.has_attachments
            });
            if let Ok(meta) = serde_json::from_value::<MessageMeta>(meta) {
                self.raw.insert(mid.clone(), raw_message(m, &spec.subject, &mid));
                self.message_rev.insert(mid.clone(), 1);
                self.messages.insert(mid, meta);
            }
        }
        self.threads.insert(spec.id, thread);
    }

    fn bump(&mut self, account: &str) -> u64 {
        let r = self.revs.entry(account.to_owned()).or_insert(1);
        *r += 1;
        *r
    }

    fn context(&self, account: &str) -> ApplyContext {
        if account == GMAIL {
            ApplyContext::gmail()
        } else {
            ApplyContext::folders(
                self.labels
                    .iter()
                    .filter(|l| l.account_id.as_str() == account && l.exclusive)
                    .filter_map(|l| l.label_key.strip_prefix("l:").map(str::to_owned)),
            )
        }
    }

    fn apply(
        &mut self,
        thread_ids: &[String],
        action: &ThreadAction,
    ) -> (BTreeMap<String, u64>, Vec<(String, Effect)>) {
        let mut revs = BTreeMap::new();
        let mut effects = Vec::new();
        for id in thread_ids {
            let Some(account) = self.threads.get(id).map(|t| t.account_id.as_str().to_owned()) else { continue };
            let ctx = self.context(&account);
            let rev = *revs.entry(account.clone()).or_insert_with(|| self.bump(&account));
            let split = self.splits.get(id).cloned();
            let Some(t) = self.threads.get_mut(id) else { continue };
            let before = Visible { keys: t.label_keys.clone(), deleted: false };
            let after = apply_action(&before, action, &ctx);
            effects.push((id.clone(), Effect::between(&before, &after)));
            let was_unread = before.keys.iter().any(|k| k == "sys:unread");
            let is_unread = after.keys.iter().any(|k| k == "sys:unread");
            t.label_keys = after.keys.clone();
            t.synced_label_keys = after.keys.clone();
            t.split =
                after.keys.iter().any(|k| k == "sys:inbox").then(|| split.unwrap_or_else(|| "split:other".into()));
            t.unread_count = u32::from(is_unread);
            if after.keys.iter().all(|k| k != "sys:snoozed") {
                t.snoozed_until = None;
            }
            t.rev = rev;
            if was_unread != is_unread {
                for m in self.messages.values_mut().filter(|m| m.thread_id.as_str() == id) {
                    m.is_unread = is_unread && m.date == t.last_message_at;
                    self.message_rev.insert(m.id.as_str().to_owned(), rev);
                }
            }
        }
        (revs, effects)
    }

    fn batch(&self, account: &str, since: u64) -> SyncChangesResult {
        let head = self.revs.get(account).copied().unwrap_or(1);
        if since > head {
            return SyncChangesResult {
                threads: vec![],
                messages: vec![],
                labels: vec![],
                tombstones: vec![],
                up_to_rev: head,
                has_more: false,
                reset_required: true,
            };
        }
        let labels = if since < 1 {
            self.labels.iter().filter(|l| l.account_id.as_str() == account).cloned().collect()
        } else {
            vec![]
        };
        SyncChangesResult {
            threads: self
                .threads
                .values()
                .filter(|t| t.account_id.as_str() == account && t.rev > since)
                .cloned()
                .collect(),
            messages: self
                .messages
                .values()
                .filter(|m| {
                    m.account_id.as_str() == account
                        && self.message_rev.get(m.id.as_str()).copied().unwrap_or(1) > since
                })
                .cloned()
                .collect(),
            labels,
            tombstones: self
                .tombstones
                .iter()
                .filter(|(a, r, _, _)| a == account && *r > since)
                .map(|(_, _, table, id)| TombstoneRef { table: *table, doc_id: id.clone() })
                .collect(),
            up_to_rev: head,
            has_more: false,
            reset_required: false,
        }
    }

    fn add_sent(&mut self, req: &SendRequest, now: i64) {
        let Some(draft) = &req.draft else { return };
        let account = draft.account_id.as_str().to_owned();
        let reply_thread = draft
            .in_reply_to_message_id
            .as_ref()
            .and_then(|m| self.messages.get(m.as_str()))
            .map(|m| m.thread_id.as_str().to_owned());
        let rev = self.bump(&account);
        let thread_id = reply_thread.unwrap_or_else(|| format!("t-sent-{}", req.client_send_id));
        let mid = format!("{thread_id}-sent-{}", req.client_send_id);
        let to: Vec<Value> = draft.to.iter().map(|a| json!({"email": a.email, "name": a.name})).collect();
        if !self.threads.contains_key(&thread_id) {
            let doc = json!({
                "_id": thread_id, "_creationTime": 1.0, "accountId": account, "providerThreadId": format!("p-{thread_id}"),
                "subject": draft.subject, "snippet": "", "participants": to, "isBulk": false, "lastMessageAt": now,
                "sortAt": now, "messageCount": 0, "unreadCount": 0, "draftCount": 0, "hasAttachments": false,
                "syncedLabelKeys": ["sys:sent"], "labelKeys": ["sys:sent"], "pendingOpCount": 0, "deletedPending": false,
                "triageState": "none", "rev": rev, "updatedAt": now
            });
            if let Ok(t) = serde_json::from_value(doc) {
                self.threads.insert(thread_id.clone(), t);
            }
        }
        let meta = json!({
            "_id": mid, "_creationTime": 1.0, "accountId": account, "threadId": thread_id, "providerMessageId": format!("p-{mid}"),
            "references": [], "from": {"name": "Me", "email": draft.identity_email}, "to": to, "cc": [], "bcc": [], "replyTo": [],
            "subject": draft.subject, "snippet": draft.text.chars().take(200).collect::<String>(), "date": now,
            "labelKeys": ["sys:sent"], "isUnread": false, "isDraft": false, "isOutbound": true, "hasAttachments": false
        });
        if let Ok(m) = serde_json::from_value::<MessageMeta>(meta) {
            let raw = format!(
                "From: Me <{}>\r\nTo: {}\r\nSubject: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n",
                draft.identity_email,
                draft.to.iter().map(|a| a.email.clone()).collect::<Vec<_>>().join(", "),
                draft.subject,
                draft.text
            );
            self.raw.insert(mid.clone(), raw.into_bytes());
            self.message_rev.insert(mid.clone(), rev);
            self.messages.insert(mid, m);
        }
        if let Some(t) = self.threads.get_mut(&thread_id) {
            t.message_count += 1;
            t.last_message_at = now;
            t.sort_at = now;
            t.snippet = draft.text.chars().take(200).collect();
            if !t.label_keys.iter().any(|k| k == "sys:sent") {
                t.label_keys.push("sys:sent".into());
                t.label_keys.sort();
                t.synced_label_keys = t.label_keys.clone();
            }
            t.rev = rev;
        }
    }
}

/// The in-process stand-in for Convex and the sync worker.
#[derive(Clone)]
pub struct DemoApi {
    state: Arc<Mutex<DemoState>>,
    changed: Arc<watch::Sender<u64>>,
}

impl std::fmt::Debug for DemoApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DemoApi")
    }
}

impl Default for DemoApi {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoApi {
    /// A fresh demo mailbox.
    pub fn new() -> Self {
        Self { state: Arc::new(Mutex::new(DemoState::new(now_ms()))), changed: Arc::new(watch::channel(0).0) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DemoState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify(&self) {
        self.changed.send_modify(|g| *g += 1);
    }

    /// Accounts (for seeding a mirror directly).
    #[cfg(test)]
    pub fn accounts(&self) -> Vec<MailAccountView> {
        self.lock().accounts.clone()
    }

    /// The full batch of an account from rev 0.
    #[cfg(test)]
    pub fn initial_batch(&self, account: &str) -> SyncChangesResult {
        self.lock().batch(account, 0)
    }
}

/// Fills a mirror with the demo data through the real feed path (tests).
#[cfg(test)]
pub fn seed_store(store: &mut Store) -> Result<(), StoreError> {
    let demo = DemoApi::new();
    store.set_accounts(&demo.accounts())?;
    for a in [GMAIL, OUTLOOK] {
        store.apply_changes(a, 0, &demo.initial_batch(a))?;
    }
    Ok(())
}

#[async_trait]
impl ClientApi for DemoApi {
    async fn register_device(&self, _args: &RegisterDeviceArgs) -> Result<(), ConvexApiError> {
        Ok(())
    }

    async fn list_accounts(&self) -> Result<Vec<MailAccountView>, ConvexApiError> {
        Ok(self.lock().accounts.clone())
    }

    async fn list_labels(&self) -> Result<Vec<LabelView>, ConvexApiError> {
        Ok(self.lock().labels.clone())
    }

    async fn mail_apply(&self, args: &MailApplyArgs) -> Result<MailApplyResult, ConvexApiError> {
        // A short delay, like a round trip.
        tokio::time::sleep(Duration::from_millis(120)).await;
        let (revs, op_ids) = {
            let mut s = self.lock();
            let ids: Vec<String> = args.thread_ids.iter().map(|t| t.as_str().to_owned()).collect();
            let (revs, effects) = s.apply(&ids, &args.action);
            s.seq += 1;
            let seq = s.seq;
            s.effects.insert(args.client_op_id.clone(), effects);
            (revs, (0..ids.len()).map(|i| format!("op-{seq}-{i}").into()).collect())
        };
        self.notify();
        Ok(MailApplyResult { op_ids, rev: revs })
    }

    async fn mail_undo(&self, args: &MailUndoArgs) -> Result<MailUndoResult, ConvexApiError> {
        let mode = {
            let mut s = self.lock();
            let effects = s.effects.remove(&args.undo_client_op_id).unwrap_or_default();
            let mut any = false;
            for (thread, effect) in effects {
                if effect.is_empty() {
                    continue;
                }
                any = true;
                s.apply(&[thread], &effect.inverse());
            }
            if any { UndoMode::Inverse } else { UndoMode::Noop }
        };
        self.notify();
        Ok(MailUndoResult { mode, op_ids: vec![] })
    }

    async fn op_failures(&self) -> Result<Subscription<Vec<OpFailure>>, ConvexApiError> {
        Ok(futures::stream::iter([Ok(Vec::new())]).chain(futures::stream::pending()).boxed())
    }

    async fn ack_failures(&self, _args: &AckFailureArgs) -> Result<(), ConvexApiError> {
        Ok(())
    }

    async fn ai_settings(&self) -> Result<Option<AiSettings>, ConvexApiError> {
        Ok(self.lock().ai.clone())
    }

    async fn save_ai_settings(&self, args: &SaveAiSettingsArgs) -> Result<(), ConvexApiError> {
        let mut s = self.lock();
        let mut cur = s.ai.clone().unwrap_or_default();
        if let Some(p) = &args.profiles {
            cur.profiles = p.clone();
        }
        if let Some(r) = &args.routes {
            cur.routes = r.clone();
        }
        if let Some(c) = &args.consent {
            cur.consent = c.clone();
        }
        cur.updated_at = now_ms();
        s.ai = Some(cur);
        Ok(())
    }

    async fn claim_triage(&self, _args: &ClaimTriageArgs) -> Result<Vec<TriageInput>, ConvexApiError> {
        Ok(Vec::new())
    }

    async fn set_triage(&self, _args: &SetTriageArgs) -> Result<(), ConvexApiError> {
        Ok(())
    }

    async fn sync_changes(&self, args: &SyncChangesArgs) -> Result<Subscription<SyncChangesResult>, ConvexApiError> {
        let account = args.account_id.as_str().to_owned();
        let since = args.since_rev;
        let state = self.state.clone();
        let rx = self.changed.subscribe();
        let stream = futures::stream::unfold((rx, true), move |(mut rx, first)| {
            let state = state.clone();
            let account = account.clone();
            async move {
                if !first && rx.changed().await.is_err() {
                    return None;
                }
                let batch = state.lock().unwrap_or_else(|e| e.into_inner()).batch(&account, since);
                Some((Ok(batch), (rx, false)))
            }
        });
        Ok(stream.boxed())
    }

    async fn list_splits(&self) -> Result<Vec<SplitView>, ConvexApiError> {
        Ok(splits())
    }

    async fn mail_snooze(&self, args: &MailSnoozeArgs) -> Result<MailSnoozeResult, ConvexApiError> {
        {
            let mut s = self.lock();
            let ids: Vec<String> = args.thread_ids.iter().map(|t| t.as_str().to_owned()).collect();
            let gmail: Vec<String> = ids
                .iter()
                .filter(|i| s.threads.get(*i).is_some_and(|t| t.account_id.as_str() == GMAIL))
                .cloned()
                .collect();
            let other: Vec<String> = ids.iter().filter(|i| !gmail.contains(i)).cloned().collect();
            s.apply(
                &gmail,
                &ThreadAction::Modify { add: vec!["sys:snoozed".into()], remove: vec!["sys:inbox".into()] },
            );
            s.apply(&other, &ThreadAction::Move { to: "sys:snoozed".into() });
            for id in &ids {
                if let Some(t) = s.threads.get_mut(id) {
                    t.snoozed_until = Some(args.until);
                }
            }
        }
        self.notify();
        Ok(MailSnoozeResult {
            timer_ids: args.thread_ids.iter().map(|t| format!("timer-{}", t.as_str()).into()).collect(),
        })
    }

    async fn set_reminder(&self, args: &MailSetReminderArgs) -> Result<MailSetReminderResult, ConvexApiError> {
        {
            let mut s = self.lock();
            let Some(account) = s.threads.get(args.thread_id.as_str()).map(|t| t.account_id.as_str().to_owned()) else {
                return Err(ConvexApiError::from_app_error(
                    "mail:setReminder",
                    "Not found".into(),
                    json!({"code": "not_found", "message": "That thread no longer exists."}),
                ));
            };
            let rev = s.bump(&account);
            if let Some(t) = s.threads.get_mut(args.thread_id.as_str()) {
                t.reminder_at = Some(args.at);
                t.rev = rev;
            }
        }
        self.notify();
        Ok(MailSetReminderResult { timer_id: format!("timer-r-{}", args.thread_id.as_str()).into() })
    }

    async fn sends_cancel(&self, args: &SendsCancelArgs) -> Result<SendsCancelResult, ConvexApiError> {
        // §10.3: a cancel that arrives before the send was scheduled leaves a tombstone.
        let mut s = self.lock();
        let id = &args.client_send_id;
        let result = if s.pending_sends.remove(id) {
            CancelOutcome::Cancelled
        } else if s.sent.contains(id) {
            CancelOutcome::TooLate
        } else {
            s.cancelled_sends.insert(id.clone());
            CancelOutcome::Cancelled
        };
        Ok(SendsCancelResult { result })
    }
}

#[async_trait]
impl MailServer for DemoApi {
    async fn raw_message(&self, message_id: &str) -> Result<Vec<u8>, WorkerError> {
        tokio::time::sleep(Duration::from_millis(60)).await;
        self.lock().raw.get(message_id).cloned().ok_or(WorkerError::Api {
            code: Some(akatsuki_core::ErrorCode::NotFound),
            message: "That message no longer exists.".into(),
            status: 404,
            version: None,
        })
    }

    async fn put_draft(&self, client_draft_id: &str, put: &DraftPut) -> Result<DraftSaved, WorkerError> {
        Ok(DraftSaved {
            draft_id: format!("draft-{client_draft_id}").into(),
            version: put.base_version + 1,
            provider_draft_id: format!("p-{client_draft_id}"),
            rfc_message_id: format!("{client_draft_id}@akatsuki-demo.example"),
        })
    }

    async fn send(&self, req: &SendRequest) -> Result<SendAccepted, WorkerError> {
        let send_at = now_ms() + i64::from(req.undo_seconds) * 1000;
        if self.lock().cancelled_sends.contains(&req.client_send_id) {
            return Ok(SendAccepted {
                send_id: format!("send-{}", req.client_send_id).into(),
                send_at,
                state: akatsuki_core::convex::worker::ScheduleOutcome::Cancelled,
            });
        }
        self.lock().pending_sends.insert(req.client_send_id.clone());
        let me = self.clone();
        let send_id = format!("send-{}", req.client_send_id);
        let req = req.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(u64::from(req.undo_seconds) * 1000)).await;
            let fire = me.lock().pending_sends.remove(&req.client_send_id);
            if fire {
                me.lock().sent.insert(req.client_send_id.clone());
                me.lock().add_sent(&req, now_ms());
                me.notify();
            }
        });
        Ok(SendAccepted {
            send_id: send_id.into(),
            send_at,
            state: akatsuki_core::convex::worker::ScheduleOutcome::Scheduled,
        })
    }

    async fn upload(
        &self,
        _filename: &str,
        _content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<akatsuki_core::wire::worker::UploadResponse, WorkerError> {
        // Nothing leaves the machine in the demo: the file is only counted.
        tokio::time::sleep(Duration::from_millis(120)).await;
        Ok(akatsuki_core::wire::worker::UploadResponse {
            upload_id: uuid::Uuid::now_v7().simple().to_string(),
            size: bytes.len() as u64,
            sha256: String::new(),
        })
    }

    async fn proxied_html(&self, _message_id: &str) -> Result<crate::remote::ProxiedHtml, WorkerError> {
        // The demo has no image proxy, so remote images stay blocked.
        Err(WorkerError::Api {
            code: Some(akatsuki_core::ErrorCode::Unsupported),
            message: "Images are not available in the demo mailbox.".into(),
            status: 400,
            version: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::store::{FeedOutcome, ListQuery};

    #[test]
    fn seeds_through_the_feed() {
        let mut store = Store::open_in_memory().unwrap();
        seed_store(&mut store).unwrap();
        let inbox = ListQuery { label_key: Some("sys:inbox".into()), ..ListQuery::default() };
        assert!(store.list_ids(&inbox).unwrap().len() > 20);
        assert!(store.messages("t-hiring").unwrap().len() == 3);
        assert_eq!(store.labels(Some(OUTLOOK)).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn apply_and_undo_round_trip_through_the_feed() {
        let api = DemoApi::new();
        let mut store = Store::open_in_memory().unwrap();
        store.set_accounts(&api.accounts()).unwrap();
        let mut sub =
            api.sync_changes(&SyncChangesArgs { account_id: GMAIL.into(), since_rev: 0, limit: 500 }).await.unwrap();
        let b = sub.next().await.unwrap().unwrap();
        assert!(matches!(store.apply_changes(GMAIL, 0, &b).unwrap(), FeedOutcome::Applied { rev: 1, .. }));

        let archive = ThreadAction::Modify { add: vec![], remove: vec!["sys:inbox".into()] };
        let r = api
            .mail_apply(&MailApplyArgs {
                client_op_id: "c1".into(),
                thread_ids: vec!["t-board".into()],
                action: archive,
            })
            .await
            .unwrap();
        assert_eq!(r.rev[GMAIL], 2);
        let b = api.lock().batch(GMAIL, 1);
        assert_eq!(b.threads.len(), 1);
        assert_eq!(b.threads[0].split, None, "archived threads leave their split");
        store.apply_changes(GMAIL, 1, &b).unwrap();
        api.mail_undo(&MailUndoArgs { client_op_id: "c2".into(), undo_client_op_id: "c1".into() }).await.unwrap();
        let b = api.lock().batch(GMAIL, 2);
        assert_eq!(b.threads[0].split.as_deref(), Some("split:vip"));
        assert!(b.threads[0].label_keys.iter().any(|k| k == "sys:inbox"));
    }

    #[tokio::test]
    async fn sends_can_be_cancelled_within_the_window() {
        let api = DemoApi::new();
        let draft = akatsuki_core::wire::worker::DraftContent {
            account_id: GMAIL.into(),
            identity_email: ME.into(),
            to: vec![akatsuki_core::Addr::new(Some("Hana"), "hana@kitsune-mail.example")],
            cc: vec![],
            bcc: vec![],
            subject: "Hello".into(),
            html: None,
            text: "Hi there".into(),
            in_reply_to_message_id: None,
            forward_of_message_id: None,
            attachments: vec![],
            read_receipt: None,
        };
        let req = SendRequest {
            client_send_id: "s1".into(),
            client_draft_id: "d1".into(),
            base_version: 0,
            draft: Some(draft),
            send_at: None,
            undo_seconds: 0,
            device_id: "dev".into(),
        };
        let _ = MailServer::send(&api, &SendRequest { client_send_id: "s0".into(), undo_seconds: 30, ..req.clone() })
            .await
            .unwrap();
        let c = api.sends_cancel(&SendsCancelArgs { client_send_id: "s0".into() }).await.unwrap();
        assert_eq!(c.result, CancelOutcome::Cancelled);
        MailServer::send(&api, &req).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(api.lock().threads.contains_key("t-sent-s1"));
        assert!(!api.lock().threads.contains_key("t-sent-s0"));
        let c = api.sends_cancel(&SendsCancelArgs { client_send_id: "s1".into() }).await.unwrap();
        assert_eq!(c.result, CancelOutcome::TooLate);
    }
}
