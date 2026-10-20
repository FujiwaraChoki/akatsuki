// Sender actions on the open thread: Unsubscribe (Ctrl/⌘ U; the `unsubscribe` op through `mail.apply`, §3.1,
// with RFC 8058 one-click POST or mailto done by the worker; a plain https link opens in a new tab instead),
// Block sender (D-286), and VIP. All in the command palette.

import { actionsFor } from "@akatsuki/mail-model";
import { Button, Dialog, errorMessage, type PaletteItem, useToast } from "@akatsuki/ui";
import { useSyncExternalStore } from "react";
import { registerExtension } from "../app/extensions.ts";
import { useBackend, useBodies } from "../backend/context.tsx";
import type { MessageBody, MessageId, ThreadDetail } from "../backend/types.ts";
import { replyTarget } from "../compose/model.ts";
import { fullDate } from "../lib/format.ts";
import { uuidv7 } from "../lib/ids.ts";
import { blockSender } from "./block.ts";

type Ask = { kind: "unsubscribe" | "block"; thread: ThreadDetail } | null;
let ask: Ask = null;
const listeners = new Set<() => void>();
function setAsk(a: Ask) {
  ask = a;
  for (const l of [...listeners]) l();
}
const useAsk = () =>
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    () => ask,
    () => ask,
  );

export type UnsubscribePlan =
  | { method: "one_click_post"; label: string }
  | {
      method: "mailto";
      label: string;
      /** The address the email goes to, shown before anything is sent. */
      to: string;
      /** Not at the sender's (or the list's) domain: the dialog warns (D-397 follow-up, §9.1). */
      foreign: boolean;
    }
  | { method: "link"; url: string; label: string }
  | null;

/** The last two labels of a domain ("lists.example.com" → "example.com"), lowercased. */
function baseDomain(d: string): string {
  return d.toLowerCase().replace(/\.+$/, "").split(".").slice(-2).join(".");
}

/** The address of a `mailto:` URL, or null. */
export function mailtoAddress(mailto: string): string | null {
  try {
    const u = new URL(mailto);
    if (u.protocol !== "mailto:") return null;
    const addr = decodeURIComponent(u.pathname).split(",")[0]!.trim().toLowerCase();
    return /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(addr) ? addr : null;
  } catch {
    return null;
  }
}

/** How to unsubscribe from the newest message's List-Unsubscribe header (D-243 reads the same message). */
export function unsubscribePlan(
  body: MessageBody | undefined,
  sender: { email?: string; listId?: string } = {},
): UnsubscribePlan {
  const lu = body?.headers.listUnsubscribe;
  if (!lu) return null;
  if (lu.oneClick && lu.http)
    return { method: "one_click_post", label: "Akatsuki asks the sender to remove you (one click)." };
  const to = lu.mailto ? mailtoAddress(lu.mailto) : null;
  if (to) {
    const target = baseDomain(to.split("@")[1]!);
    const own = [sender.email?.split("@")[1], sender.listId?.replace(/^.*<|>$/g, "")]
      .filter((d): d is string => Boolean(d))
      .map(baseDomain);
    const foreign = own.length > 0 && !own.includes(target);
    return {
      method: "mailto",
      to,
      foreign,
      label: foreign
        ? `Akatsuki would email ${to} from your account to ask to be removed. That address is not at the sender's domain: only continue if you trust this message.`
        : `Akatsuki emails ${to} from your account to ask to be removed.`,
    };
  }
  if (lu.http && /^https:\/\//i.test(lu.http))
    return { method: "link", url: lu.http, label: "The sender's unsubscribe page opens in a new tab." };
  return null;
}

function SenderDialog() {
  const a = useAsk();
  const backend = useBackend();
  const bodies = useBodies();
  const m = backend.useMutations();
  const rules = backend.useRules() ?? [];
  const labels = backend.useLabels() ?? [];
  const { toast } = useToast();
  if (!a) return null;
  const last = replyTarget(a.thread.messages);
  const sender = last?.from?.email.toLowerCase() ?? null;
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  if (a.kind === "block") {
    const domain = sender ? `@${sender.split("@")[1]}` : null;
    const block = (who: string) => {
      setAsk(null);
      void blockSender(who, true, { m, rules })
        .then(() =>
          m.apply({ clientOpId: uuidv7(), threadIds: [a.thread.thread._id], action: { kind: "trash" } }),
        )
        .then(() => toast({ title: `Blocked ${who}. New mail from them goes to Trash.` }))
        .catch(fail);
    };
    return (
      <Dialog
        open
        onOpenChange={(o) => !o && setAsk(null)}
        title="Block sender"
        size="sm"
        description="This conversation moves to Trash, and so will new mail from them."
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              Cancel
            </Button>
            {domain && (
              <Button onClick={() => block(domain)} disabled={!sender}>
                Block all of {domain}
              </Button>
            )}
            <Button variant="danger" disabled={!sender} onClick={() => sender && block(sender)}>
              Block {sender ?? "sender"}
            </Button>
          </>
        }
      />
    );
  }
  const plan = unsubscribePlan(last ? bodies.cache.peek(last._id) : undefined, {
    email: last?.from?.email,
    listId: last?.listId,
  });
  const helpers = actionsFor(
    a.thread.account,
    labels.filter((l) => l.accountId === a.thread.account._id),
  );
  const archiveAction = helpers.available("archive") ? helpers.archive() : null;
  const go = (archive: boolean) => {
    setAsk(null);
    if (!plan) return;
    if (plan.method === "link") {
      window.open(plan.url, "_blank", "noopener,noreferrer");
    } else
      void m
        .apply({
          clientOpId: uuidv7(),
          threadIds: [a.thread.thread._id],
          action: { kind: "unsubscribe", method: plan.method },
        })
        .then(() => toast({ title: "Unsubscribe requested." }))
        .catch(fail);
    if (archive && archiveAction)
      void m
        .apply({ clientOpId: uuidv7(), threadIds: [a.thread.thread._id], action: archiveAction })
        .catch(fail);
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => !o && setAsk(null)}
      title={`Unsubscribe from ${last?.from?.name ?? sender ?? "this sender"}`}
      size="sm"
      description={
        plan
          ? plan.label
          : "This message has no unsubscribe link. Block the sender instead to send their mail to Trash."
      }
      footer={
        plan ? (
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              Cancel
            </Button>
            <Button onClick={() => go(false)}>Unsubscribe</Button>
            {archiveAction && (
              <Button variant="primary" onClick={() => go(true)}>
                Unsubscribe and archive
              </Button>
            )}
          </>
        ) : (
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              Close
            </Button>
            <Button variant="primary" onClick={() => setAsk({ kind: "block", thread: a.thread })}>
              Block sender
            </Button>
          </>
        )
      }
    />
  );
}

/** Opens of the user's own messages in this thread, only when read receipts are on (decision 8). */
function ReadReceiptsFooter({ thread }: { thread: ThreadDetail }) {
  const backend = useBackend();
  const me = backend.useMe();
  if (!me?.profile.prefs.readReceipts.enabled) return null;
  const mine = thread.messages.filter((x) => x.isOutbound && !x.isDraft);
  if (mine.length === 0) return null;
  return (
    <ul className="ak-thread__receipts" aria-label="Read receipts">
      {mine.map((x) => (
        <Receipt key={x._id} messageId={x._id} date={x.date} tz={me.profile.timezone} />
      ))}
    </ul>
  );
}

function Receipt({ messageId, date, tz }: { messageId: MessageId; date: number; tz: string }) {
  const backend = useBackend();
  const r = backend.useReadReceipt(messageId);
  if (!r) return null;
  const opens = r.opens;
  const last = opens[opens.length - 1];
  return (
    <li className="ak-thread__timer">
      Your message of {fullDate(date, tz)}:{" "}
      {opens.length === 0
        ? "no opens recorded"
        : `opened ${opens.length === 1 ? "once" : `${opens.length} times`}, last ${fullDate(last!.at, tz)}`}
      {opens.some((o) => o.proxied) && " (some opens came from a mail privacy proxy and may not be a person)"}
      .
    </li>
  );
}

export function registerSenders(): void {
  registerExtension({
    id: "senders",
    overlay: SenderDialog,
    threadFooter: ReadReceiptsFooter,
    commands: (ctx) =>
      ctx.openThread
        ? { "thread.unsubscribe": () => setAsk({ kind: "unsubscribe", thread: ctx.openThread! }) }
        : {},
    paletteItems: (ctx) => {
      const t = ctx.openThread;
      if (!t) return [];
      const items: PaletteItem[] = [
        {
          id: "sender:block",
          label: "Block sender",
          section: "Conversation",
          keywords: ["spam", "mute", "trash"],
          run: () => setAsk({ kind: "block", thread: t }),
        },
      ];
      return items;
    },
  });
}
