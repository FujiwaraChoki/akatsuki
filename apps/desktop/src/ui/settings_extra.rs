//! Settings pages for labels and snippets, and the AI page's feature switches (auto-triage,
//! dictation and where its audio goes).

use gpui::{AnyElement, AppContext, Context, Entity, Focusable, FontWeight, Window, div, prelude::*, px};

use akatsuki_core::convex::AiFeature;

use super::input::TextField;
use super::settings::{group, group_title, row};
use super::widgets::{button, field_frame, segment, switch};
use super::workspace::Workspace;
use crate::theme::styles;

/// Text fields of the settings forms.
pub struct Forms {
    /// New label name.
    pub label_name: Entity<TextField>,
    /// Account index of the new label.
    pub label_account: usize,
    /// The label being renamed, and its field.
    pub renaming: Option<(String, Entity<TextField>)>,
    /// The label waiting for a second click to delete.
    pub confirm_delete: Option<String>,
    /// Snippet being edited (`None` = new).
    pub snippet_id: Option<String>,
    /// Snippet name.
    pub snippet_name: Entity<TextField>,
    /// `;shortcut`.
    pub snippet_shortcut: Entity<TextField>,
    /// Subject.
    pub snippet_subject: Entity<TextField>,
    /// Body (plain text; `{{first_name}}` and friends).
    pub snippet_body: Entity<TextField>,
    /// The snippet waiting for a second click to delete.
    pub confirm_snippet_delete: Option<String>,
}

impl Forms {
    /// Empty forms.
    pub fn new(cx: &mut Context<'_, Workspace>) -> Self {
        let f = |cx: &mut Context<'_, Workspace>, p: &str| cx.new(|cx| TextField::new(p, cx));
        Self {
            label_name: f(cx, "New label name"),
            label_account: 0,
            renaming: None,
            confirm_delete: None,
            snippet_id: None,
            snippet_name: f(cx, "Name, e.g. Thanks"),
            snippet_shortcut: f(cx, "Shortcut, e.g. thx (type ;thx then Tab)"),
            snippet_subject: f(cx, "Subject (optional, fills an empty subject)"),
            snippet_body: cx.new(|cx| TextField::new("Hi {{first_name}}, …", cx).multiline(8)),
            confirm_snippet_delete: None,
        }
    }
}

fn field(t: &crate::theme::Theme, f: &Entity<TextField>, window: &Window, cx: &gpui::App) -> gpui::Div {
    field_frame(t, f.focus_handle(cx).is_focused(window)).child(div().flex_1().child(f.clone()))
}

impl Workspace {
    pub(crate) fn labels_page(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let (accounts, labels) = {
            let store = self.store();
            (store.accounts().unwrap_or_default(), store.labels(None).unwrap_or_default())
        };
        let mut page = div().flex().flex_col().child(div().pt(px(12.)).text_color(t.tertiary).child(
            "Labels live in your mailbox (Gmail labels, Outlook categories, IMAP keywords). Folders come from the provider.",
        ));
        // New label.
        let mut account_row = div().flex().flex_wrap().gap(px(4.));
        if accounts.len() > 1 {
            for (i, a) in accounts.iter().enumerate() {
                account_row = account_row.child(
                    segment(&t, ("label-account", i), a.email.clone(), self.forms.label_account == i).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.forms.label_account = i;
                            cx.notify();
                        }),
                    ),
                );
            }
        }
        page = page.child(group_title("New label")).child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(16.))
                .rounded(px(akatsuki_tokens::radius::LG))
                .border_1()
                .border_color(t.line)
                .when(accounts.len() > 1, |d| d.child(account_row))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .items_center()
                        .child(field(&t, &self.forms.label_name, window, cx).flex_1())
                        .child(button(&t, "label-create", "Create", true).on_click(cx.listener(|this, _, _, cx| {
                            let name = this.forms.label_name.read(cx).content.clone();
                            let accounts = this.store().accounts().unwrap_or_default();
                            if let Some(a) =
                                accounts.get(this.forms.label_account.min(accounts.len().saturating_sub(1)))
                            {
                                this.backend.create_label(a.id.as_str().to_owned(), name, Vec::new());
                                this.forms.label_name.update(cx, |f, cx| f.set_text("", cx));
                            }
                            cx.notify();
                        }))),
                ),
        );
        for a in &accounts {
            let mine: Vec<_> = labels
                .iter()
                .filter(|l| l.account_id == a.id && l.role.is_none() && !l.hidden && l.label_key.starts_with("l:"))
                .cloned()
                .collect();
            page = page.child(group_title(&a.email));
            let mut rows: Vec<AnyElement> = Vec::new();
            if mine.is_empty() {
                rows.push(
                    div().px(px(16.)).py(px(14.)).text_color(t.tertiary).child("No labels yet.").into_any_element(),
                );
            }
            for (i, l) in mine.into_iter().enumerate() {
                let id = l.id.as_str().to_owned();
                let editing = self.forms.renaming.as_ref().filter(|(r, _)| *r == id).map(|(_, f)| f.clone());
                let confirming = self.forms.confirm_delete.as_deref() == Some(id.as_str());
                let key = format!("{}-{i}", a.id.as_str());
                let mut r = div().flex().items_center().gap(px(10.)).min_h(px(52.)).px(px(16.)).py(px(8.));
                r = match editing {
                    Some(f) => {
                        let (id2, f2) = (id.clone(), f.clone());
                        r.child(field(&t, &f, window, cx).flex_1()).child(
                            button(&t, gpui::SharedString::from(format!("label-save-{key}")), "Save", true).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    let name = f2.read(cx).content.clone();
                                    this.backend.rename_label(&id2, name);
                                    this.forms.renaming = None;
                                    cx.notify();
                                }),
                            ),
                        )
                    }
                    None => {
                        let (id2, name) = (id.clone(), l.name.clone());
                        r.child(
                            div()
                                .flex_1()
                                .child(l.name.clone())
                                .when(l.pending_delete, |d| d.text_color(t.muted).child(" (deleting…)")),
                        )
                        .child(
                            button(&t, gpui::SharedString::from(format!("label-rename-{key}")), "Rename", false)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let f = cx.new(|cx| TextField::new("Label name", cx));
                                    f.update(cx, |x, cx| x.set_text(&name, cx));
                                    window.focus(&f.focus_handle(cx));
                                    this.forms.renaming = Some((id2.clone(), f));
                                    cx.notify();
                                })),
                        )
                    }
                };
                let (id3, name3) = (id.clone(), l.name.clone());
                r = r.child(
                    button(
                        &t,
                        gpui::SharedString::from(format!("label-delete-{key}")),
                        if confirming { "Confirm delete" } else { "Delete" },
                        false,
                    )
                    .text_color(t.accent_text)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.forms.confirm_delete.as_deref() == Some(id3.as_str()) {
                            this.backend.remove_label(&id3, &name3);
                            this.forms.confirm_delete = None;
                        } else {
                            this.forms.confirm_delete = Some(id3.clone());
                        }
                        cx.notify();
                    })),
                );
                rows.push(r.into_any_element());
            }
            page = page.child(group(&t, rows));
        }
        page.into_any_element()
    }

    pub(crate) fn snippets_page(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let mut list: Vec<AnyElement> = Vec::new();
        if self.snippets.is_empty() {
            list.push(
                div().px(px(16.)).py(px(14.)).text_color(t.tertiary).child("No snippets yet.").into_any_element(),
            );
        }
        for (i, s) in crate::vm::snippets::ordered(&self.snippets).into_iter().enumerate() {
            let id = s.id.as_str().to_owned();
            let confirming = self.forms.confirm_snippet_delete.as_deref() == Some(id.as_str());
            let preview: String =
                super::reading::one_line(&crate::vm::snippets::html_to_text(&s.body_html)).chars().take(90).collect();
            let edit = s.clone();
            let id2 = id.clone();
            list.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(div().font_weight(FontWeight::MEDIUM).child(s.name.clone()))
                                    .when_some(s.shortcut.clone(), |d, sc| {
                                        d.child(div().text_color(t.muted).child(format!(";{sc}")))
                                    }),
                            )
                            .child(
                                div().truncate().text_size(px(styles::META.size)).text_color(t.muted).child(preview),
                            ),
                    )
                    .child(button(&t, ("snippet-edit", i), "Edit", false).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let body = crate::vm::snippets::html_to_text(&edit.body_html);
                            this.forms.snippet_id = Some(edit.id.as_str().to_owned());
                            let f = &this.forms;
                            let vals = [
                                (f.snippet_name.clone(), edit.name.clone()),
                                (f.snippet_shortcut.clone(), edit.shortcut.clone().unwrap_or_default()),
                                (f.snippet_subject.clone(), edit.subject.clone().unwrap_or_default()),
                                (f.snippet_body.clone(), body),
                            ];
                            for (e, v) in vals {
                                e.update(cx, |x, cx| x.set_text(&v, cx));
                            }
                            cx.notify();
                        },
                    )))
                    .child(
                        button(&t, ("snippet-delete", i), if confirming { "Confirm delete" } else { "Delete" }, false)
                            .text_color(t.accent_text)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.forms.confirm_snippet_delete.as_deref() == Some(id2.as_str()) {
                                    this.backend.remove_snippet(&id2);
                                    this.forms.confirm_snippet_delete = None;
                                } else {
                                    this.forms.confirm_snippet_delete = Some(id2.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
        }
        let editing = self.forms.snippet_id.is_some();
        let km = self.settings.keymap.keymap();
        let hint = crate::keys::hint(akatsuki_shortcuts::ids::COMPOSE_SNIPPET, km).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().pt(px(12.)).text_color(t.tertiary).child(format!(
                "Insert a snippet with {hint} in the composer, or type ;shortcut then Tab. {{{{first_name}}}}, {{{{name}}}}, {{{{email}}}}, {{{{day}}}} and {{{{date}}}} fill in by themselves."
            )))
            .child(group_title("Your snippets"))
            .child(group(&t, list))
            .child(group_title(if editing { "Edit snippet" } else { "New snippet" }))
            .child(field(&t, &self.forms.snippet_name, window, cx))
            .child(field(&t, &self.forms.snippet_shortcut, window, cx))
            .child(field(&t, &self.forms.snippet_subject, window, cx))
            .child(field(&t, &self.forms.snippet_body, window, cx).min_h(px(120.)).items_start().py(px(8.)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(button(&t, "snippet-save", "Save snippet", true).on_click(cx.listener(|this, _, _, cx| {
                        let f = &this.forms;
                        let read = |e: &Entity<TextField>| e.read(cx).content.clone();
                        let (name, shortcut, subject, body) =
                            (read(&f.snippet_name), read(&f.snippet_shortcut), read(&f.snippet_subject), read(&f.snippet_body));
                        if name.trim().is_empty() || body.trim().is_empty() {
                            this.toast("A snippet needs a name and some text", false, cx);
                            return;
                        }
                        this.backend.save_snippet(f.snippet_id.clone(), name, Some(shortcut), Some(subject), body);
                        this.clear_snippet_form(cx);
                    })))
                    .when(editing, |d| {
                        d.child(
                            button(&t, "snippet-cancel", "Cancel", false)
                                .on_click(cx.listener(|this, _, _, cx| this.clear_snippet_form(cx))),
                        )
                    }),
            )
            .into_any_element()
    }

    fn clear_snippet_form(&mut self, cx: &mut Context<'_, Self>) {
        self.forms.snippet_id = None;
        let f = &self.forms;
        for e in [f.snippet_name.clone(), f.snippet_shortcut.clone(), f.snippet_subject.clone(), f.snippet_body.clone()]
        {
            e.update(cx, |x, cx| x.set_text("", cx));
        }
        cx.notify();
    }

    /// The AI page's feature rows: auto-triage on this device, and where dictation audio goes.
    pub(crate) fn ai_features_rows(&mut self, cx: &mut Context<'_, Self>) -> AnyElement {
        let t = self.theme;
        let settings = self.ai_settings.clone().unwrap_or_default();
        let auto = settings.consent.auto_triage;
        let triage_egress = self.backend.ai_egress(AiFeature::Triage);
        let triage_detail = match &triage_egress {
            Some(e) => format!("Sorts new inbox mail into categories and marks what needs a reply. {e}."),
            None => "Sorts new inbox mail into categories. Set up a provider first.".to_owned(),
        };
        let dictation = match self.backend.dictation_ready() {
            Ok(e) => format!("Dictate into the composer. {e}."),
            Err(e) => format!("Dictate into the composer. {e}"),
        };
        group(
            &t,
            vec![
                row(
                    &t,
                    "Triage new mail",
                    &triage_detail,
                    switch(&t, "auto-triage", auto).on_click(cx.listener(move |this, _, _, cx| {
                        this.backend.set_auto_triage(!auto);
                        cx.notify();
                    })),
                )
                .into_any_element(),
                row(&t, "Dictation", &dictation, div()).into_any_element(),
            ],
        )
        .into_any_element()
    }
}
