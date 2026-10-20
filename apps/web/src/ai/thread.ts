// Message text for AI features and reply quotes: the bodies come from the device's body cache (fetched from the
// worker once per thread, §7.2). Quoted history is stripped with the worker's offsets (UTF-8 bytes, D-251),
// and HTML-only messages are reduced to text. Nothing here leaves the device; the caller decides.
import { splitQuoted } from "@akatsuki/wire";
import type { MessageBody, MessageMeta, ThreadDetail } from "../backend/types.ts";
import { htmlToText } from "../compose/text.ts";
import type { BodyLoader } from "../mail/bodyCache.ts";

/** The visible (unquoted) text of a body; the snippet when there is none. */
export function visibleText(body: MessageBody | undefined, meta: Pick<MessageMeta, "snippet">): string {
  if (!body) return meta.snippet;
  if (body.text) return splitQuoted(body.text, body.quoted.textStart).visible.trim() || body.text.trim();
  if (body.html) return htmlToText(splitQuoted(body.html, body.quoted.htmlStart).visible);
  return meta.snippet;
}

/** Waits for the thread's bodies (at most `timeoutMs`), then returns what is cached. */
export async function ensureBodies(
  loader: BodyLoader,
  detail: ThreadDetail,
  timeoutMs = 8000,
): Promise<void> {
  const ids = detail.messages.map((m) => m._id);
  await Promise.race([
    loader.loadThread(detail.thread._id, ids),
    new Promise((r) => setTimeout(r, timeoutMs)),
  ]);
}

/** `ThreadForTranscript` (packages/ai) for summarize, instant replies, extract actions and compose. */
export function threadForAi(detail: ThreadDetail, loader: BodyLoader, timezone: string) {
  return {
    timezone,
    messages: detail.messages
      .filter((m) => !m.isDraft)
      .map((m) => ({
        from: m.from ?? null,
        to: m.to,
        cc: m.cc,
        date: m.date,
        text: visibleText(loader.cache.peek(m._id), m),
      })),
  };
}
