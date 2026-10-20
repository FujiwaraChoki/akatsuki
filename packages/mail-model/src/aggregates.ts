// Thread keys and aggregates (ARCHITECTURE §3.3, §4.4 step 3). Computed from at most the newest 200
// messages inside worker.upsertThreads. Rust twin of threadKeysFromMessages: crates/core::model::aggregates.

import { sortDedupe } from "./applyOps.ts";
import { SYS } from "./labelKeys.ts";
import type { Addr } from "./types.ts";

/** Hard caps (§1.6). */
export const CAPS = {
  subject: 998,
  snippet: 200,
  participants: 10,
  addrList: 100,
  references: 20,
  searchText: 1500,
  threadsPerApply: 500,
  aggregateMessages: 200,
} as const;

/** Truncates to `n` UTF-16 code units (Convex string lengths are measured the same way). */
export const truncate = (s: string, n: number): string => (s.length > n ? s.slice(0, n) : s);

/** The per-message facts aggregates are computed from. */
export interface MessageFacts {
  from?: Addr;
  to: Addr[];
  cc: Addr[];
  subject: string;
  snippet: string;
  date: number;
  labelKeys: string[];
  isUnread: boolean;
  isDraft: boolean;
  isOutbound: boolean;
  hasAttachments: boolean;
  listId?: string;
}

/** Union of the messages' keys; `sys:unread` iff at least one message is unread. Sorted, deduped. */
export function threadKeysFromMessages(
  messages: readonly Pick<MessageFacts, "labelKeys" | "isUnread">[],
): string[] {
  const keys = new Set<string>();
  for (const m of messages) for (const k of m.labelKeys) keys.add(k);
  keys.delete(SYS.unread);
  if (messages.some((m) => m.isUnread)) keys.add(SYS.unread);
  return sortDedupe([...keys]);
}

export interface ThreadAggregates {
  subject: string;
  snippet: string;
  participants: Addr[];
  lastFrom?: Addr;
  lastMessageAt: number;
  lastInboundAt?: number;
  lastOutboundAt?: number;
  messageCount: number;
  unreadCount: number;
  draftCount: number;
  hasAttachments: boolean;
  isBulk: boolean;
  listId?: string;
}

/**
 * Aggregates over (at most) the newest 200 messages of a thread. Drafts count only when the thread has
 * nothing else; participants are the newest-first unique addresses (lowercased), at most 10.
 */
export function threadAggregates(all: readonly MessageFacts[]): ThreadAggregates {
  const messages = [...all].sort((a, b) => a.date - b.date).slice(-CAPS.aggregateMessages);
  const sent = messages.filter((m) => !m.isDraft);
  const base = sent.length > 0 ? sent : messages;
  const latest = base[base.length - 1];
  const first = base[0];
  const participants: Addr[] = [];
  const seen = new Set<string>();
  for (const m of [...base].reverse()) {
    for (const a of [m.from, ...m.to, ...m.cc]) {
      if (!a) continue;
      const email = a.email.trim().toLowerCase();
      if (seen.has(email)) continue;
      seen.add(email);
      participants.push(a.name ? { name: a.name, email } : { email });
      if (participants.length >= CAPS.participants) break;
    }
    if (participants.length >= CAPS.participants) break;
  }
  const maxDate = (ms: readonly MessageFacts[]) =>
    ms.length ? Math.max(...ms.map((m) => m.date)) : undefined;
  const listId = [...base].reverse().find((m) => m.listId)?.listId;
  return {
    subject: truncate(first?.subject || latest?.subject || "", CAPS.subject),
    snippet: truncate(latest?.snippet ?? "", CAPS.snippet),
    participants,
    lastFrom: latest?.from,
    lastMessageAt: maxDate(base) ?? 0,
    lastInboundAt: maxDate(sent.filter((m) => !m.isOutbound)),
    lastOutboundAt: maxDate(sent.filter((m) => m.isOutbound)),
    messageCount: sent.length,
    unreadCount: messages.filter((m) => m.isUnread).length,
    draftCount: messages.filter((m) => m.isDraft).length,
    hasAttachments: messages.some((m) => m.hasAttachments),
    isBulk: listId !== undefined,
    listId,
  };
}

/** `threadSearch.text`: subject, participant names and emails, and the last 3 snippets; at most 1,500 chars. */
export function threadSearchText(
  subject: string,
  participants: readonly Addr[],
  snippets: readonly string[],
): string {
  const people = participants.map((p) => (p.name ? `${p.name} ${p.email}` : p.email)).join(" ");
  return truncate([subject, people, ...snippets.slice(-3)].join("\n"), CAPS.searchText);
}
