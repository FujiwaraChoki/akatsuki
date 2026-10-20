// The reading pane: an open thread (ARCHITECTURE §4.2 `mail.getThread`, §7.2 bodies, §14.3 rendering).
// Headers come live from Convex; bodies stream from the worker once per thread and are cached on the device.
import { SYS } from "@akatsuki/mail-model";
import { Avatar, Badge, Button, Heading, Spinner } from "@akatsuki/ui";
import { splitQuoted } from "@akatsuki/wire";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { TriageChip } from "../ai/TriageChip.tsx";
import { extensions } from "../app/extensions.ts";
import { useBackend, useBodies, useSyncOrigin } from "../backend/context.tsx";
import type { LabelView, MessageBody, MessageMeta, Prefs, ThreadDetail } from "../backend/types.ts";
import { ErrorState } from "../components/ErrorState.tsx";
import { IconImage, IconPaperclip } from "../components/Icons.tsx";
import { displayName, fileSize, fullDate } from "../lib/format.ts";
import { useTheme } from "../lib/theme.ts";
import type { BodyLoader, BodyState } from "./bodyCache.ts";
import { isPlainLooking, prepareHtml, textParagraphs } from "./bodyHtml.ts";
import { MessageFrame } from "./MessageFrame.tsx";

/** Body states for the messages of a thread; starts (or joins) the thread's body stream. */
export function useThreadBodies(
  loader: BodyLoader,
  detail: ThreadDetail | null | undefined,
): { states: Map<string, BodyState>; retry: () => void } {
  const threadId = detail?.thread._id ?? null;
  const ids = useMemo(() => detail?.messages.map((m) => m._id) ?? [], [detail?.messages]);
  const key = ids.join(",");
  const [attempt, setAttempt] = useState(0);
  const [, setTick] = useState(0);
  useEffect(() => {
    const off = loader.subscribe(() => setTick((t) => t + 1));
    return () => {
      off();
    };
  }, [loader]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for `ids`
  useEffect(() => {
    if (threadId && ids.length > 0) void loader.loadThread(threadId, ids, { force: attempt > 0 });
  }, [loader, threadId, key, attempt]);
  const states = new Map<string, BodyState>();
  const failed = threadId ? loader.errorFor(threadId) : undefined;
  for (const id of ids) {
    const body = loader.cache.peek(id);
    if (body) states.set(id, { status: "ready", body });
    else if (!threadId || loader.isLoading(threadId)) states.set(id, { status: "loading" });
    else if (failed !== undefined) states.set(id, { status: "error", error: failed });
    else if (loader.finished(threadId)) states.set(id, { status: "error", error: { code: "not_found" } });
    else states.set(id, { status: "loading" });
  }
  return { states, retry: () => setAttempt((a) => a + 1) };
}

export interface ThreadViewProps {
  detail: ThreadDetail;
  labelsByKey: ReadonlyMap<string, LabelView>;
  timezone: string;
  prefs: Prefs;
  multiAccount: boolean;
  /** Bumped by the "Show remote images" command. */
  showImagesTick: number;
  /** Bumped by the "Expand all messages" command (shift+x). */
  expandAllTick: number;
  toolbar: ReactNode;
}

export function ThreadView({
  detail,
  labelsByKey,
  timezone,
  prefs,
  multiAccount,
  showImagesTick,
  expandAllTick,
  toolbar,
}: ThreadViewProps) {
  const loader = useBodies();
  const { states: bodies, retry } = useThreadBodies(loader, detail);
  const { thread, messages, account } = detail;
  const last = messages[messages.length - 1];
  const [expanded, setExpanded] = useState<Set<string>>(() => defaultExpanded(messages));
  const [showImages, setShowImages] = useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: reset per thread only
  useEffect(() => {
    setExpanded(defaultExpanded(messages));
    setShowImages(false);
  }, [thread._id]);
  // A new message arriving while the thread is open expands it (collapsing the last one stays possible).
  const lastId = last?._id;
  useEffect(() => {
    if (lastId) setExpanded((s) => (s.has(lastId) ? s : new Set([...s, lastId])));
  }, [lastId]);
  useEffect(() => {
    if (showImagesTick > 0) setShowImages(true);
  }, [showImagesTick]);
  useEffect(() => {
    if (expandAllTick > 0) setExpanded(new Set(messages.map((m) => m._id)));
  }, [expandAllTick, messages]);

  const labels = thread.labelKeys
    .map((k) => labelsByKey.get(k))
    .filter(
      (l): l is LabelView => l !== undefined && !l.hidden && l.kind !== "system" && l.role === undefined,
    );
  const footers = extensions().filter((e) => e.threadFooter);

  return (
    <article className="ak-thread" aria-labelledby={`subject-${thread._id}`}>
      <header className="ak-thread__header">
        <div className="ak-thread__toolbar">{toolbar}</div>
        <Heading level={1} variant="subject" id={`subject-${thread._id}`} className="ak-thread__subject">
          {thread.subject || "(no subject)"}
        </Heading>
        <div className="ak-thread__meta">
          {thread.labelKeys.includes(SYS.starred) && <Badge tone="accent">Starred</Badge>}
          <TriageChip triage={thread.triage} />
          {thread.snoozedUntil && (
            <Badge tone="neutral">Snoozed until {fullDate(thread.snoozedUntil, timezone)}</Badge>
          )}
          {labels.map((l) => (
            <Badge key={l._id} tone="outline">
              {l.name}
            </Badge>
          ))}
          {multiAccount && <span className="ak-thread__account">{account.email}</span>}
          {thread.pendingOpCount > 0 && <span className="ak-thread__pending">Saving…</span>}
        </div>
      </header>
      {detail.truncated && (
        <p className="ak-thread__truncated">Older messages in this conversation are not shown.</p>
      )}
      <ol className="ak-thread__messages">
        {messages.map((m) => (
          <li key={m._id}>
            <MessageView
              meta={m}
              state={bodies.get(m._id) ?? { status: "loading" }}
              expanded={expanded.has(m._id)}
              onToggle={() =>
                setExpanded((s) => {
                  const n = new Set(s);
                  if (n.has(m._id)) n.delete(m._id);
                  else n.add(m._id);
                  return n;
                })
              }
              timezone={timezone}
              prefs={prefs}
              showImages={showImages}
              onShowImages={() => setShowImages(true)}
              onRetry={retry}
            />
          </li>
        ))}
      </ol>
      {footers.map((e) => {
        const Footer = e.threadFooter!;
        return <Footer key={e.id} thread={detail} />;
      })}
    </article>
  );
}

function defaultExpanded(messages: MessageMeta[]): Set<string> {
  const s = new Set(messages.filter((m) => m.isUnread).map((m) => m._id));
  const last = messages[messages.length - 1];
  if (last) s.add(last._id);
  return s;
}

interface MessageViewProps {
  meta: MessageMeta;
  state: BodyState;
  expanded: boolean;
  onToggle: () => void;
  timezone: string;
  prefs: Prefs;
  showImages: boolean;
  onShowImages: () => void;
  onRetry?: () => void;
}

function MessageView({
  meta,
  state,
  expanded,
  onToggle,
  timezone,
  prefs,
  showImages,
  onShowImages,
  onRetry,
}: MessageViewProps) {
  const from = meta.from;
  const recipients = [...meta.to, ...meta.cc].map((a) => displayName(a)).join(", ");
  return (
    <section
      className="ak-message"
      data-expanded={expanded || undefined}
      aria-label={`Message from ${displayName(from)}`}
    >
      <button type="button" className="ak-message__head" onClick={onToggle} aria-expanded={expanded}>
        <Avatar name={from?.name} email={from?.email ?? "?"} size="md" />
        <span className="ak-message__who">
          <span className="ak-message__from">{displayName(from)}</span>
          {expanded ? (
            <span className="ak-message__to">to {recipients || "undisclosed recipients"}</span>
          ) : (
            <span className="ak-message__snippet">{meta.snippet}</span>
          )}
        </span>
        <time className="ak-message__date" dateTime={new Date(meta.date).toISOString()}>
          {fullDate(meta.date, timezone)}
        </time>
      </button>
      {expanded && (
        <div className="ak-message__body">
          <Body
            meta={meta}
            state={state}
            prefs={prefs}
            showImages={showImages}
            onShowImages={onShowImages}
            onRetry={onRetry}
          />
        </div>
      )}
    </section>
  );
}

function Body({
  meta,
  state,
  prefs,
  showImages,
  onShowImages,
  onRetry,
}: {
  meta: MessageMeta;
  state: BodyState;
  prefs: Prefs;
  showImages: boolean;
  onShowImages: () => void;
  onRetry?: () => void;
}) {
  if (state.status === "loading")
    return (
      <div className="ak-message__loading">
        <Spinner size="sm" label="Loading message" />
        <span className="ak-message__skeleton" />
        <span className="ak-message__skeleton ak-message__skeleton--short" />
      </div>
    );
  if (state.status === "error") return <ErrorState error={state.error} onRetry={onRetry} inline />;
  return (
    <ReadyBody
      meta={meta}
      body={state.body}
      prefs={prefs}
      showImages={showImages}
      onShowImages={onShowImages}
    />
  );
}

function ReadyBody({
  meta,
  body,
  prefs,
  showImages,
  onShowImages,
}: {
  meta: MessageMeta;
  body: MessageBody;
  prefs: Prefs;
  showImages: boolean;
  onShowImages: () => void;
}) {
  const backend = useBackend();
  const syncOrigin = useSyncOrigin();
  const sender = meta.from?.email ?? null;
  const senderPref = backend.useSenderPref(sender);
  const m = backend.useMutations();
  const [showQuoted, setShowQuoted] = useState(false);
  const [cidImages, setCidImages] = useState<Map<string, string>>(new Map());
  const allowImages = showImages || prefs.remoteImages === "proxy" || senderPref?.allowRemoteImages === true;

  const split = useMemo(
    () => (body.html ? splitQuoted(body.html, body.quoted.htmlStart) : null),
    [body.html, body.quoted.htmlStart],
  );
  const html = split ? (showQuoted ? `${split.visible}${split.quoted}` : split.visible) : null;
  const prepared = useMemo(
    () => (html !== null ? prepareHtml(html, { showImages: allowImages, syncOrigin, cidImages }) : null),
    [html, allowImages, syncOrigin, cidImages],
  );

  // Inline (cid) images: fetched with the bearer token, injected as data: URLs (§14.3).
  const missing = prepared?.missingCids.join(",") ?? "";
  useEffect(() => {
    if (!missing) return;
    let cancelled = false;
    const ctrl = new AbortController();
    (async () => {
      const found = new Map<string, string>();
      for (const partId of missing.split(",")) {
        try {
          const blob = await backend.worker.getPartBlob(body.messageId, partId, ctrl.signal);
          if (!/^image\/(png|gif|jpeg|webp)$/.test(blob.type)) continue;
          found.set(partId, await blobToDataUrl(blob));
        } catch {
          /* leave the placeholder */
        }
      }
      if (!cancelled && found.size > 0) setCidImages((prev) => new Map([...prev, ...found]));
    })();
    return () => {
      cancelled = true;
      ctrl.abort();
    };
  }, [missing, body.messageId, backend.worker]);

  const blocked = prepared?.blockedImages ?? 0;
  const theme = useTheme();
  const plainLooking = useMemo(() => (prepared ? isPlainLooking(prepared.html) : false), [prepared]);
  return (
    <>
      {blocked > 0 && !allowImages && (
        <div className="ak-images-banner" role="note">
          <IconImage />
          <span>
            {blocked === 1 ? "1 image is" : `${blocked} images are`} hidden to protect your privacy.
            {body.trackersRemoved > 0 &&
              ` ${body.trackersRemoved} tracker${body.trackersRemoved === 1 ? " was" : "s were"} removed.`}
          </span>
          <Button size="sm" variant="secondary" shortcut="thread.showImages" onClick={onShowImages}>
            Show images
          </Button>
          {sender && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void m.setSenderPref({ sender, allowRemoteImages: true })}
            >
              Always from this sender
            </Button>
          )}
        </div>
      )}
      {prepared ? (
        <MessageFrame
          html={prepared.html}
          syncOrigin={syncOrigin}
          title={`Message from ${displayName(meta.from)}`}
          mode={theme.mode === "dark" && plainLooking ? "dark" : "light"}
        />
      ) : (
        <div className="ak-textbody">
          {textParagraphs(body.text ?? "").map((p, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: paragraphs are positional
            <p key={i}>{p}</p>
          ))}
        </div>
      )}
      {split?.quoted && (
        <button
          type="button"
          className="ak-quoted-toggle"
          aria-expanded={showQuoted}
          onClick={() => setShowQuoted((s) => !s)}
        >
          {showQuoted ? "Hide quoted text" : "Show quoted text"}
        </button>
      )}
      {body.attachments.filter((a) => !a.inline).length > 0 && (
        <ul className="ak-attachments" aria-label="Attachments">
          {body.attachments
            .filter((a) => !a.inline)
            .map((a) => (
              <li key={a.partId}>
                <button
                  type="button"
                  className="ak-attachment"
                  onClick={async () => {
                    const url = await backend.worker.createLink(body.messageId, a.partId);
                    const link = document.createElement("a");
                    link.href = url;
                    link.download = a.filename;
                    link.rel = "noopener noreferrer";
                    link.click();
                  }}
                >
                  <IconPaperclip />
                  <span className="ak-attachment__name">{a.filename}</span>
                  <span className="ak-attachment__size">{fileSize(a.size)}</span>
                </button>
              </li>
            ))}
        </ul>
      )}
    </>
  );
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error);
    r.readAsDataURL(blob);
  });
}
