// Blocking a sender (D-286). `senderPrefs.blocked` records the choice, but nothing on the server acts on it yet,
// so the web client also keeps a filter rule "Blocked: <sender>" (`from_email` / `from_domain` → trash) that
// Convex applies to new inbound mail (§4.3 step 5). Unblocking clears both.
import type { Mutations, RuleView } from "../backend/types.ts";
import { BLOCK_RULE_PREFIX } from "../settings/ruleModel.ts";

export async function blockSender(
  sender: string,
  blocked: boolean,
  deps: { m: Pick<Mutations, "setSenderPref" | "saveRule" | "removeRule">; rules: readonly RuleView[] },
): Promise<void> {
  const s = sender.trim().toLowerCase();
  await deps.m.setSenderPref({ sender: s, blocked });
  const existing = deps.rules.filter((r) => r.name === `${BLOCK_RULE_PREFIX}${s}`);
  if (!blocked) {
    for (const r of existing) await deps.m.removeRule({ ruleId: r._id });
    return;
  }
  if (existing.length > 0) return;
  const cond = s.startsWith("@")
    ? ({ f: "from_domain", eq: s.slice(1) } as const)
    : ({ f: "from_email", eq: s } as const);
  await deps.m.saveRule({
    name: `${BLOCK_RULE_PREFIX}${s}`,
    enabled: true,
    order: -1,
    rule: { match: "all", conds: [cond] },
    actions: [{ kind: "trash" }],
  });
}
