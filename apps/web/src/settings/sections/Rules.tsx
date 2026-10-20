// Settings: the split inbox editor (§3.4) and filter rules (§3.2 `rules`, applied by Convex to newly arrived
// inbound mail). Both use the structured condition language (`vRuleCond`): pick a field, then a value; match
// all or any; at most 20 conditions. Splits are reordered with the arrow buttons; a split
// shows how many of the newest inbox conversations it would take. Alt ↑ / Alt ↓ on a split's Edit button moves it.
import { matchRule, SYS } from "@akatsuki/mail-model";
import { Badge, Button, errorMessage, IconButton, useToast } from "@akatsuki/ui";
import { type KeyboardEvent, useMemo, useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { LabelView, MailAccountView, Rule, RuleView, SplitView } from "../../backend/types.ts";
import { IconChevron, IconClose } from "../../components/Icons.tsx";
import { visibleLabels } from "../../mail/Sidebar.tsx";
import {
  BLOCK_RULE_PREFIX,
  COND_FIELDS,
  type CondField,
  condText,
  defaultCond,
  fieldInfo,
  fromThreadAction,
  MAX_CONDS,
  MAX_SPLITS,
  normaliseRule,
  type RuleCond,
  ruleProblem,
  TRIAGE_CATEGORIES,
  toThreadAction,
  UI_ACTIONS,
  type UiAction,
  withText,
} from "../ruleModel.ts";
import { Empty, Row, Section, Segmented, Switch } from "../ui.tsx";

interface Ctx {
  labels: readonly LabelView[];
  accounts: readonly MailAccountView[];
}

export function ConditionEditor({
  rule,
  onChange,
  ctx,
}: {
  rule: Rule;
  onChange: (r: Rule) => void;
  ctx: Ctx;
}) {
  const setCond = (i: number, c: RuleCond) =>
    onChange({ ...rule, conds: rule.conds.map((x, j) => (j === i ? c : x)) });
  const labelOptions = [
    { key: SYS.starred, name: "Starred" },
    { key: SYS.important, name: "Important" },
    { key: SYS.unread, name: "Unread" },
    ...visibleLabels(ctx.labels).map((l) => ({
      key: l.labelKey,
      name:
        ctx.accounts.length > 1
          ? `${l.name} (${ctx.accounts.find((a) => a._id === l.accountId)?.email ?? ""})`
          : l.name,
    })),
  ];
  return (
    <div className="ak-rule">
      <Row label="Match">
        <Segmented
          label="Match"
          value={rule.match}
          options={[
            { value: "all", label: "All conditions" },
            { value: "any", label: "Any condition" },
          ]}
          onChange={(match) => onChange({ ...rule, match })}
        />
      </Row>
      <ol className="ak-rule__conds">
        {rule.conds.map((c, i) => {
          const info = fieldInfo(c.f);
          const text = condText(c);
          return (
            // biome-ignore lint/suspicious/noArrayIndexKey: conditions are positional
            <li key={i} className="ak-rule__cond">
              <select
                className="ak-input ak-input--select"
                aria-label={`Condition ${i + 1} field`}
                value={c.f}
                onChange={(e) => setCond(i, defaultCond(e.target.value as CondField, ctx, c))}
              >
                {COND_FIELDS.map((f) => (
                  <option key={f.f} value={f.f}>
                    {f.label}
                  </option>
                ))}
              </select>
              {text !== null && (
                <input
                  className="ak-input"
                  aria-label={`Condition ${i + 1} value`}
                  placeholder={info.placeholder}
                  value={text}
                  onChange={(e) => setCond(i, withText(c, e.target.value))}
                />
              )}
              {info.kind === "bool" && "eq" in c && typeof c.eq === "boolean" && (
                <select
                  className="ak-input ak-input--select"
                  aria-label={`Condition ${i + 1} value`}
                  value={c.eq ? "yes" : "no"}
                  onChange={(e) => setCond(i, { ...c, eq: e.target.value === "yes" } as RuleCond)}
                >
                  <option value="yes">is true</option>
                  <option value="no">is false</option>
                </select>
              )}
              {c.f === "label" && (
                <select
                  className="ak-input ak-input--select"
                  aria-label={`Condition ${i + 1} value`}
                  value={c.key}
                  onChange={(e) => setCond(i, { f: "label", key: e.target.value })}
                >
                  {labelOptions.map((l) => (
                    <option key={l.key} value={l.key}>
                      {l.name}
                    </option>
                  ))}
                </select>
              )}
              {c.f === "triage" && (
                <select
                  className="ak-input ak-input--select"
                  aria-label={`Condition ${i + 1} value`}
                  value={c.category}
                  onChange={(e) => setCond(i, { f: "triage", category: e.target.value as never })}
                >
                  {TRIAGE_CATEGORIES.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </select>
              )}
              {c.f === "account" && (
                <select
                  className="ak-input ak-input--select"
                  aria-label={`Condition ${i + 1} value`}
                  value={c.accountId}
                  onChange={(e) => setCond(i, { f: "account", accountId: e.target.value as never })}
                >
                  {ctx.accounts.map((a) => (
                    <option key={a._id} value={a._id}>
                      {a.email}
                    </option>
                  ))}
                </select>
              )}
              <IconButton
                label={`Remove condition ${i + 1}`}
                icon={<IconClose />}
                size="sm"
                onClick={() => onChange({ ...rule, conds: rule.conds.filter((_, j) => j !== i) })}
              />
            </li>
          );
        })}
      </ol>
      <Button
        size="sm"
        variant="ghost"
        disabled={rule.conds.length >= MAX_CONDS}
        onClick={() => onChange({ ...rule, conds: [...rule.conds, defaultCond("from_domain", ctx)] })}
      >
        Add condition
      </Button>
    </div>
  );
}

// ───────────── splits ─────────────

export function SplitsSection() {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const raw = backend.useSplits();
  const labels = backend.useLabels() ?? [];
  const accounts = backend.useAccounts() ?? [];
  const inbox = backend.useThreadList({ labelKey: SYS.inbox });
  const splits = useMemo(() => [...(raw ?? [])].sort((a, b) => a.order - b.order), [raw]);
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });

  const move = (i: number, d: -1 | 1) => {
    const j = i + d;
    if (j < 0 || j >= splits.length) return;
    const order = splits.map((s) => s._id);
    [order[i], order[j]] = [order[j]!, order[i]!];
    void m.reorderSplits({ order }).catch(fail);
  };

  const matches = (rule: Rule) =>
    inbox.rows.filter((r) =>
      matchRule(rule as never, {
        ...(r.lastFrom && { fromEmail: r.lastFrom.email }),
        participants: r.participants.map((p) => p.email),
        labelKeys: r.labelKeys,
        ...(r.triage && { triage: r.triage }),
        isBulk: r.isBulk,
        subject: r.subject,
        accountId: r.accountId,
        vip: false,
      }),
    ).length;

  return (
    <Section
      title="Split inbox"
      description="Splits are tabs over your inbox. A conversation goes to the first split whose rule matches, else to Other. Tab and Shift Tab move between them."
      actions={
        <Button variant="primary" disabled={splits.length >= MAX_SPLITS} onClick={() => setEditing("new")}>
          New split
        </Button>
      }
    >
      {editing === "new" && (
        <SplitForm
          ctx={{ labels, accounts }}
          matches={matches}
          onCancel={() => setEditing(null)}
          onSave={(a) =>
            m
              .saveSplit(a)
              .then(() => {
                setEditing(null);
                toast({ title: `Split “${a.name}” created.` });
              })
              .catch(fail)
          }
        />
      )}
      {raw === undefined ? null : splits.length === 0 ? (
        <Empty>No splits. Your inbox is one list.</Empty>
      ) : (
        <ol className="ak-settings__cards">
          {splits.map((s, i) =>
            editing === s._id ? (
              <li key={s._id}>
                <SplitForm
                  split={s}
                  ctx={{ labels, accounts }}
                  matches={matches}
                  onCancel={() => setEditing(null)}
                  onSave={(a) =>
                    m
                      .saveSplit({ ...a, splitId: s._id })
                      .then(() => setEditing(null))
                      .catch(fail)
                  }
                />
              </li>
            ) : (
              <li key={s._id} className="ak-settings__card ak-split" aria-label={`Split ${s.name}`}>
                <span className="ak-split__order">{i + 1}</span>
                <div className="ak-split__text">
                  <span className="ak-settings__card-title">{s.name}</span>
                  <span className="ak-settings__hint">{describeRule(s.rule, { labels, accounts })}</span>
                </div>
                {s.showCount && <Badge tone="outline">Count</Badge>}
                <IconButton
                  label={`Move ${s.name} up`}
                  icon={<IconChevron className="ak-rot-up" />}
                  size="sm"
                  disabled={i === 0}
                  onClick={() => move(i, -1)}
                />
                <IconButton
                  label={`Move ${s.name} down`}
                  icon={<IconChevron className="ak-rot-down" />}
                  size="sm"
                  disabled={i === splits.length - 1}
                  onClick={() => move(i, 1)}
                />
                <Button
                  size="sm"
                  onClick={() => setEditing(s._id)}
                  onKeyDown={(e: KeyboardEvent) => {
                    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
                      e.preventDefault();
                      move(i, e.key === "ArrowUp" ? -1 : 1);
                    }
                  }}
                >
                  Edit
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() =>
                    void m
                      .removeSplit({ splitId: s._id })
                      .then(() =>
                        toast({ title: `Split “${s.name}” deleted. Its mail is back in the inbox tabs.` }),
                      )
                      .catch(fail)
                  }
                >
                  Delete
                </Button>
              </li>
            ),
          )}
        </ol>
      )}
    </Section>
  );
}

function SplitForm({
  split,
  ctx,
  matches,
  onSave,
  onCancel,
}: {
  split?: SplitView;
  ctx: Ctx;
  matches: (r: Rule) => number;
  onSave: (a: { name: string; rule: Rule; showCount: boolean }) => Promise<unknown>;
  onCancel: () => void;
}) {
  const [name, setName] = useState(split?.name ?? "");
  const [rule, setRule] = useState<Rule>(
    split?.rule ?? { match: "all", conds: [defaultCond("from_domain", ctx)] },
  );
  const [showCount, setShowCount] = useState(split?.showCount ?? true);
  const [busy, setBusy] = useState(false);
  const problem = ruleProblem(name, rule);
  const [touched, setTouched] = useState(false);
  const n = problem ? null : matches(normaliseRule(rule));
  return (
    <form
      className="ak-settings__card ak-rule-form"
      aria-label={split ? `Edit split ${split.name}` : "New split"}
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (problem) return;
        setBusy(true);
        void onSave({ name: name.trim(), rule: normaliseRule(rule), showCount }).finally(() =>
          setBusy(false),
        );
      }}
    >
      <Row label="Name" htmlFor="split-name">
        <input
          id="split-name"
          className="ak-input"
          value={name}
          // biome-ignore lint/a11y/noAutofocus: the first field of a form the user just opened
          autoFocus
          onChange={(e) => setName(e.target.value)}
        />
      </Row>
      <ConditionEditor rule={rule} onChange={setRule} ctx={ctx} />
      <Row label="Show unread count on the tab">
        <Switch label="Show unread count" checked={showCount} onChange={setShowCount} />
      </Row>
      {n !== null && (
        <p className="ak-settings__hint" aria-live="polite">
          Matches {n} of your newest inbox conversations.
        </p>
      )}
      {touched && problem && (
        <p className="ak-compose__error" role="alert">
          {problem}
        </p>
      )}
      <div className="ak-settings__actions">
        <Button type="submit" variant="primary" loading={busy}>
          Save split
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

export function describeRule(rule: Rule, ctx: Ctx): string {
  const parts = rule.conds.map((c) => {
    const info = fieldInfo(c.f);
    const t = condText(c);
    if (t !== null) return `${info.label.toLowerCase()} ${t}`;
    if (c.f === "label")
      return `label ${ctx.labels.find((l) => l.labelKey === c.key)?.name ?? c.key.replace("sys:", "")}`;
    if (c.f === "triage") return `AI category ${c.category}`;
    if (c.f === "account") return `account ${ctx.accounts.find((a) => a._id === c.accountId)?.email ?? ""}`;
    return `${info.label.toLowerCase()}${"eq" in c && c.eq === false ? " (no)" : ""}`;
  });
  return parts.join(rule.match === "all" ? " and " : " or ") || "No conditions";
}

// ───────────── filter rules ─────────────

export function RulesSection() {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const rules = backend.useRules();
  const labels = backend.useLabels() ?? [];
  const accounts = backend.useAccounts() ?? [];
  const [editing, setEditing] = useState<string | "new" | null>(null);
  const sorted = [...(rules ?? [])].sort((a, b) => a.order - b.order);
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  return (
    <Section
      title="Rules"
      description="Rules act on new mail as it arrives: archive, label, star, move or trash it. They never run on older mail."
      actions={
        <Button variant="primary" disabled={accounts.length === 0} onClick={() => setEditing("new")}>
          New rule
        </Button>
      }
    >
      {editing === "new" && (
        <RuleForm
          ctx={{ labels, accounts }}
          order={sorted.length}
          onCancel={() => setEditing(null)}
          onSave={(a) =>
            m
              .saveRule(a)
              .then(() => {
                setEditing(null);
                toast({ title: `Rule “${a.name}” saved.` });
              })
              .catch(fail)
          }
        />
      )}
      {rules === undefined ? null : sorted.length === 0 ? (
        <Empty>No rules yet.</Empty>
      ) : (
        <ul className="ak-settings__cards">
          {sorted.map((r) =>
            editing === r._id ? (
              <li key={r._id}>
                <RuleForm
                  rule={r}
                  ctx={{ labels, accounts }}
                  order={r.order}
                  onCancel={() => setEditing(null)}
                  onSave={(a) =>
                    m
                      .saveRule({ ...a, ruleId: r._id })
                      .then(() => setEditing(null))
                      .catch(fail)
                  }
                />
              </li>
            ) : (
              <li key={r._id} className="ak-settings__card ak-split">
                <Switch
                  label={`Rule ${r.name} enabled`}
                  checked={r.enabled}
                  onChange={(enabled) =>
                    void m
                      .saveRule({
                        ruleId: r._id,
                        ...(r.accountId && { accountId: r.accountId }),
                        name: r.name,
                        enabled,
                        order: r.order,
                        rule: r.rule,
                        actions: r.actions,
                      })
                      .catch(fail)
                  }
                />
                <div className="ak-split__text">
                  <span className="ak-settings__card-title">{r.name}</span>
                  <span className="ak-settings__hint">
                    When {describeRule(r.rule, { labels, accounts })}:{" "}
                    {r.actions.map((a) => describeAction(fromThreadAction(a, labels), labels)).join(", ")}
                    {r.accountId
                      ? ` (${accounts.find((a) => a._id === r.accountId)?.email ?? "one account"})`
                      : " (all accounts)"}
                  </span>
                </div>
                {!r.name.startsWith(BLOCK_RULE_PREFIX) && (
                  <Button size="sm" onClick={() => setEditing(r._id)}>
                    Edit
                  </Button>
                )}
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => void m.removeRule({ ruleId: r._id }).catch(fail)}
                >
                  Delete
                </Button>
              </li>
            ),
          )}
        </ul>
      )}
    </Section>
  );
}

function describeAction(a: UiAction, labels: readonly LabelView[]): string {
  if (a.type === "label") return `add label ${labels.find((l) => l._id === a.labelId)?.name ?? ""}`;
  if (a.type === "move")
    return `move to ${labels.find((l) => l.labelKey === a.folderKey)?.name ?? a.folderKey}`;
  if (a.type === "custom") return "a custom action";
  return UI_ACTIONS.find((x) => x.type === a.type)!.label.toLowerCase();
}

function RuleForm({
  rule: existing,
  ctx,
  order,
  onSave,
  onCancel,
}: {
  rule?: RuleView;
  ctx: Ctx;
  order: number;
  onSave: (a: {
    accountId?: MailAccountView["_id"];
    name: string;
    enabled: boolean;
    order: number;
    rule: Rule;
    actions: RuleView["actions"];
  }) => Promise<unknown>;
  onCancel: () => void;
}) {
  const [name, setName] = useState(existing?.name ?? "");
  const [accountId, setAccountId] = useState<string>(existing?.accountId ?? ctx.accounts[0]?._id ?? "");
  const [rule, setRule] = useState<Rule>(
    existing?.rule ?? { match: "all", conds: [defaultCond("from_email", ctx)] },
  );
  const [actions, setActions] = useState<UiAction[]>(
    existing?.actions.map((a) => fromThreadAction(a, ctx.labels)) ?? [{ type: "archive" }],
  );
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const account = ctx.accounts.find((a) => a._id === accountId);
  const accLabels = ctx.labels.filter((l) => l.accountId === accountId);
  const folders = accLabels.filter((l) => l.exclusive && l.role !== "drafts" && l.role !== "sent");
  const userLabels = visibleLabels(accLabels).filter((l) => !l.exclusive);
  let builtActions: RuleView["actions"] | null = null;
  let buildProblem: string | null = null;
  if (!account) buildProblem = "Choose an account.";
  else
    try {
      builtActions = actions.map((a) => toThreadAction(a, account, ctx.labels));
    } catch (e) {
      buildProblem = errorMessage(e);
    }
  const problem = ruleProblem(name, rule) ?? buildProblem ?? (actions.length === 0 ? "Add an action." : null);
  const setAction = (i: number, a: UiAction) => setActions((xs) => xs.map((x, j) => (j === i ? a : x)));
  return (
    <form
      className="ak-settings__card ak-rule-form"
      aria-label={existing ? `Edit rule ${existing.name}` : "New rule"}
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (problem || !builtActions) return;
        setBusy(true);
        void onSave({
          accountId: accountId as never,
          name: name.trim(),
          enabled: existing?.enabled ?? true,
          order,
          rule: normaliseRule(rule),
          actions: builtActions,
        }).finally(() => setBusy(false));
      }}
    >
      <Row label="Name" htmlFor="rule-name">
        <input
          id="rule-name"
          className="ak-input"
          value={name}
          // biome-ignore lint/a11y/noAutofocus: the first field of a form the user just opened
          autoFocus
          onChange={(e) => setName(e.target.value)}
        />
      </Row>
      <Row
        label="Account"
        htmlFor="rule-account"
        hint="Rules act on one mailbox, so labels and folders are that mailbox's."
      >
        <select
          id="rule-account"
          className="ak-input ak-input--select"
          value={accountId}
          onChange={(e) => setAccountId(e.target.value)}
        >
          {ctx.accounts.map((a) => (
            <option key={a._id} value={a._id}>
              {a.email}
            </option>
          ))}
        </select>
      </Row>
      <ConditionEditor rule={rule} onChange={setRule} ctx={ctx} />
      <h3 className="ak-ai__sub">Then</h3>
      <ol className="ak-rule__conds">
        {actions.map((a, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: positional
          <li key={i} className="ak-rule__cond">
            <select
              className="ak-input ak-input--select"
              aria-label={`Action ${i + 1}`}
              value={a.type}
              onChange={(e) => {
                const type = e.target.value as UiAction["type"];
                if (type === "label") setAction(i, { type, labelId: userLabels[0]?._id ?? "" });
                else if (type === "move")
                  setAction(i, { type, folderKey: folders[0]?.labelKey ?? "sys:inbox" });
                else if (type !== "custom") setAction(i, { type });
              }}
            >
              {UI_ACTIONS.filter(
                (x) =>
                  (x.type !== "move" || folders.length > 0) && (x.type !== "label" || userLabels.length > 0),
              ).map((x) => (
                <option key={x.type} value={x.type}>
                  {x.label}
                </option>
              ))}
              {a.type === "custom" && <option value="custom">Custom action</option>}
            </select>
            {a.type === "label" && (
              <select
                className="ak-input ak-input--select"
                aria-label={`Action ${i + 1} label`}
                value={a.labelId}
                onChange={(e) => setAction(i, { type: "label", labelId: e.target.value })}
              >
                {userLabels.map((l) => (
                  <option key={l._id} value={l._id}>
                    {l.name}
                  </option>
                ))}
              </select>
            )}
            {a.type === "move" && (
              <select
                className="ak-input ak-input--select"
                aria-label={`Action ${i + 1} folder`}
                value={a.folderKey}
                onChange={(e) => setAction(i, { type: "move", folderKey: e.target.value })}
              >
                {folders.map((l) => (
                  <option key={l._id} value={l.labelKey}>
                    {l.name}
                  </option>
                ))}
              </select>
            )}
            <IconButton
              label={`Remove action ${i + 1}`}
              icon={<IconClose />}
              size="sm"
              onClick={() => setActions((xs) => xs.filter((_, j) => j !== i))}
            />
          </li>
        ))}
      </ol>
      <Button
        size="sm"
        variant="ghost"
        disabled={actions.length >= 5}
        onClick={() => setActions((xs) => [...xs, { type: "read" }])}
      >
        Add action
      </Button>
      {touched && problem && (
        <p className="ak-compose__error" role="alert">
          {problem}
        </p>
      )}
      <div className="ak-settings__actions">
        <Button type="submit" variant="primary" loading={busy}>
          Save rule
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
