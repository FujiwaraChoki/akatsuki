// AI in the mail client (ARCHITECTURE §12): under an open thread, Summarize (Shift S), Suggest replies (Shift R)
// and Extract actions, each with its egress label; "Search with AI" turns a sentence into the one search
// grammar (§11) shown as editable chips before it runs; client-side triage for BYO and local routes while the
// tab is in the foreground (§12.5). Every action is in the command palette.
import {
  Button,
  Dialog,
  Eyebrow,
  errorMessage,
  Kbd,
  type PaletteItem,
  Spinner,
  useToast,
} from "@akatsuki/ui";
import { useCallback, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { registerExtension } from "../app/extensions.ts";
import { navigate } from "../app/router.tsx";
import { useBackend, useBodies } from "../backend/context.tsx";
import type { MeView, ThreadDetail, ThreadId } from "../backend/types.ts";
import { composer } from "../compose/store.ts";
import { deviceId } from "../lib/ids.ts";
import { parseSnoozeInput } from "../mail/snoozeTimes.ts";
import { openReminder } from "../reminders/extension.tsx";
import { EgressNote } from "./Egress.tsx";
import { loadAi, preloadAi } from "./module.ts";
import { ensureBodies, threadForAi } from "./thread.ts";
import { STATUS_COPY, type UseAi, useAi, useAiStream } from "./useAi.ts";
import { voiceFor } from "./voice.ts";

type ThreadAiKind = "summarize" | "instant_reply" | "extract_actions";

// One-shot requests from shortcuts and the palette to the footer of the open thread.
let threadRequest: { kind: ThreadAiKind; threadId: string; seq: number; at: number } | null = null;
let seq = 0;
const reqListeners = new Set<() => void>();
function requestThreadAi(kind: ThreadAiKind, threadId: string) {
  threadRequest = { kind, threadId, seq: ++seq, at: Date.now() };
  for (const l of [...reqListeners]) l();
}
const subscribeReq = (cb: () => void) => {
  reqListeners.add(cb);
  return () => {
    reqListeners.delete(cb);
  };
};

let nlOpen = false;
function setNlOpen(v: boolean) {
  nlOpen = v;
  for (const l of [...reqListeners]) l();
}

const todayIn = (tz: string) =>
  new Intl.DateTimeFormat("en-CA", {
    timeZone: tz,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(new Date());

interface Summary {
  tldr?: string;
  bullets?: string[];
  asks?: string[];
  deadlines?: { text?: string; due?: string | null }[];
}
interface Replies {
  replies?: { label?: string; body?: string }[];
}
interface Actions {
  actions?: { kind?: string; title?: string; due?: string | null; quote?: string }[];
}

function ThreadAiFooter({ thread }: { thread: ThreadDetail }) {
  return <ThreadAi key={thread.thread._id} detail={thread} />;
}

function ThreadAi({ detail }: { detail: ThreadDetail }) {
  const ai = useAi();
  const backend = useBackend();
  const me = backend.useMe();
  const loader = useBodies();
  const summary = useAiStream(ai);
  const replies = useAiStream(ai);
  const actions = useAiStream(ai);
  const tz = me?.profile.timezone ?? "UTC";
  const userName = me?.name ?? me?.email ?? "";
  const threadId = detail.thread._id;

  const run = useCallback(
    async (kind: ThreadAiKind) => {
      const mod = ai.ai ?? (await loadAi());
      await ensureBodies(loader, detail);
      const t = threadForAi(detail, loader, tz);
      if (kind === "summarize")
        void summary.start("summarize", mod.summarizeVars({ thread: t, userName }), { cache: true });
      else if (kind === "instant_reply")
        void replies.start(
          "instant_reply",
          mod.instantReplyVars({ thread: t, userName, voice: voiceFor(ai.settings) }),
          { cache: true },
        );
      else void actions.start("extract_actions", mod.extractActionsVars({ thread: t }), { cache: true });
    },
    [ai, loader, detail, tz, userName, summary, replies, actions],
  );

  const req = useSyncExternalStore(
    subscribeReq,
    () => threadRequest,
    () => threadRequest,
  );
  // Requests made before this footer mounted belong to an earlier visit of a thread: never replay them.
  const handled = useRef(threadRequest?.seq ?? 0);
  const { toast } = useToast();
  useEffect(() => {
    if (!req || req.seq === handled.current || req.threadId !== threadId) return;
    handled.current = req.seq;
    if (Date.now() - req.at > 2000) return;
    // A key or palette request while the feature cannot run says why (and how to fix it) instead of doing
    // nothing (review round 2).
    const status = ai.status(req.kind);
    if (status !== "ready" && status !== "loading") {
      toast({
        id: "ai-unavailable",
        title: STATUS_COPY[status],
        action: { label: "Open AI settings", onAction: () => navigate("/settings/ai") },
      });
      return;
    }
    void run(req.kind);
  }, [req, threadId, run, ai, toast]);

  const consent = ai.status("summarize");
  if (consent === "loading") return null;
  if (consent === "consent")
    return (
      <section className="ak-ai ak-ai--off" aria-label="AI">
        <EgressNote ai={ai} feature="summarize" />
      </section>
    );
  const s = summary.state.object as Summary | null;
  const r = replies.state.object as Replies | null;
  const a = actions.state.object as Actions | null;
  return (
    <section className="ak-ai" aria-label="AI">
      <div className="ak-ai__bar" role="toolbar" aria-label="AI actions">
        <Button
          size="sm"
          variant="ghost"
          shortcut="ai.summarize"
          onClick={() => void run("summarize")}
          loading={summary.state.status === "running"}
        >
          Summarize
        </Button>
        <Button
          size="sm"
          variant="ghost"
          shortcut="ai.instantReply"
          onClick={() => void run("instant_reply")}
          loading={replies.state.status === "running"}
        >
          Suggest replies
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void run("extract_actions")}
          loading={actions.state.status === "running"}
        >
          Extract actions
        </Button>
      </div>
      <EgressNote ai={ai} feature="summarize" />

      {(summary.state.status === "running" || summary.state.status === "done") && s && (
        <div className="ak-ai__card" aria-live="polite">
          <Eyebrow>Summary</Eyebrow>
          {s.tldr && <p className="ak-ai__tldr">{s.tldr}</p>}
          {s.bullets && s.bullets.length > 0 && (
            <ul className="ak-ai__list">
              {s.bullets.slice(0, 5).map((b, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: streamed list, positional
                <li key={i}>{b}</li>
              ))}
            </ul>
          )}
          {s.asks && s.asks.length > 0 && (
            <>
              <h3 className="ak-ai__sub">Asks</h3>
              <ul className="ak-ai__list">
                {s.asks.map((x, i) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: positional
                  <li key={i}>{x}</li>
                ))}
              </ul>
            </>
          )}
          {s.deadlines && s.deadlines.length > 0 && (
            <>
              <h3 className="ak-ai__sub">Deadlines</h3>
              <ul className="ak-ai__list">
                {s.deadlines.map((x, i) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: positional
                  <li key={i}>
                    {x.text}
                    {x.due ? ` (${x.due})` : ""}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
      {summary.state.status === "error" && <AiError error={summary.state.error} />}

      {r?.replies && r.replies.length > 0 && (
        <fieldset className="ak-ai__replies">
          <legend className="ak-visually-hidden">Suggested replies</legend>
          {r.replies.slice(0, 3).map((x, i) => (
            <button
              // biome-ignore lint/suspicious/noArrayIndexKey: positional
              key={i}
              type="button"
              className="ak-ai__reply"
              disabled={replies.state.status !== "done" || !x.body}
              onClick={() => composer.requestReply(threadId as ThreadId, "reply", x.body ?? "")}
            >
              <span className="ak-ai__reply-label">{x.label}</span>
              <span className="ak-ai__reply-body">{x.body}</span>
            </button>
          ))}
        </fieldset>
      )}
      {replies.state.status === "error" && <AiError error={replies.state.error} />}

      {a?.actions && (
        <div className="ak-ai__card">
          <Eyebrow>Actions</Eyebrow>
          {a.actions.length === 0 ? (
            <p className="ak-settings__hint">Nothing to do in this conversation.</p>
          ) : (
            <ul className="ak-ai__actions">
              {a.actions.map((x, i) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: positional
                <li key={i} className="ak-ai__action">
                  <span className="ak-ai__action-kind">{x.kind}</span>
                  <span className="ak-ai__action-title">
                    {x.title}
                    {x.due ? ` (${x.due})` : ""}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={actions.state.status !== "done"}
                    onClick={() =>
                      openReminder({
                        threadId: threadId as ThreadId,
                        note: x.title ?? "",
                        ...(x.due &&
                          parseSnoozeInput(x.due, Date.now(), tz) !== null && {
                            at: parseSnoozeInput(x.due, Date.now(), tz)!,
                          }),
                      })
                    }
                  >
                    Remind me
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {actions.state.status === "error" && <AiError error={actions.state.error} />}
    </section>
  );
}

function AiError({ error }: { error: unknown }) {
  return (
    <p className="ak-compose__error" role="alert">
      {errorMessage(error)}
    </p>
  );
}

/** Splits a grammar string into chips, keeping quoted phrases together. */
export function queryChips(q: string): string[] {
  return [...q.matchAll(/(?:-?[a-z_]+:)?"(?:[^"\\]|\\.)*"|\S+/gi)].map((m) => m[0]);
}

function NlSearchDialog({ me }: { me: MeView }) {
  const open = useSyncExternalStore(
    subscribeReq,
    () => nlOpen,
    () => nlOpen,
  );
  const ai = useAi();
  const backend = useBackend();
  const labels = backend.useLabels();
  const splits = backend.useSplits();
  const stream = useAiStream(ai);
  const [sentence, setSentence] = useState("");
  const [query, setQuery] = useState("");
  const id = useId();
  useEffect(() => {
    if (open) {
      setSentence("");
      setQuery("");
      stream.cancel();
    }
  }, [open, stream.cancel]);
  const out = stream.state.object as { query?: string; explanation?: string } | null;
  useEffect(() => {
    if (stream.state.status === "done" && out?.query) setQuery(out.query);
  }, [stream.state.status, out?.query]);
  const ask = () => {
    if (!sentence.trim()) return;
    const tz = me.profile.timezone;
    void stream.start("nl_search", {
      query: sentence.trim(),
      today: todayIn(tz),
      timezone: tz,
      labels: (labels ?? [])
        .filter((l) => l.kind !== "system")
        .map((l) => l.name)
        .join("\n"),
      splits: (splits ?? []).map((s) => s.name).join("\n"),
    });
  };
  const runSearch = () => {
    if (!query.trim()) return;
    setNlOpen(false);
    navigate(`/search?q=${encodeURIComponent(query.trim())}`);
  };
  const chips = queryChips(query);
  return (
    <Dialog open={open} onOpenChange={(o) => !o && setNlOpen(false)} title="Search with AI" size="md">
      <label className="ak-field__label" htmlFor={id}>
        Describe what you are looking for
      </label>
      <div className="ak-compose__ai-row">
        <input
          id={id}
          className="ak-input"
          value={sentence}
          placeholder="unread mail from Mara with a PDF last week"
          onChange={(e) => setSentence(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              ask();
            }
          }}
        />
        <Button
          variant="primary"
          onClick={ask}
          loading={stream.state.status === "running"}
          disabled={ai.status("nl_search") !== "ready"}
        >
          Translate
        </Button>
      </div>
      <EgressNote ai={ai} feature="nl_search" />
      {stream.state.status === "error" && <AiError error={stream.state.error} />}
      {stream.state.status === "running" && <Spinner size="sm" label="Translating" />}
      {(query || out?.query) && (
        <div className="ak-nl">
          {out?.explanation && <p className="ak-settings__hint">{out.explanation}</p>}
          <ul className="ak-nl__chips" aria-label="Search operators">
            {chips.map((c, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: chips may repeat
              <li key={`${c}-${i}`} className="ak-chip">
                <span className="ak-chip__text">{c}</span>
                <button
                  type="button"
                  className="ak-chip__remove"
                  aria-label={`Remove ${c}`}
                  onClick={() => setQuery(chips.filter((_, j) => j !== i).join(" "))}
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
          <label className="ak-field__label" htmlFor={`${id}-q`}>
            Search
          </label>
          <input
            id={`${id}-q`}
            className="ak-input ak-input--mono"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                runSearch();
              }
            }}
          />
          <div className="ak-dialog__actions">
            <Button variant="primary" onClick={runSearch} disabled={!query.trim()}>
              Search <Kbd keys="enter" />
            </Button>
          </div>
        </div>
      )}
    </Dialog>
  );
}

/** BYO / local triage while this tab is visible (§12.5); managed triage runs in the worker instead. */
function TriageRunner() {
  const ai = useAi();
  const backend = useBackend();
  const m = backend.useMutations();
  const enabled = ai.settings.consent.autoTriage && ai.status("triage") === "ready";
  const managed = useMemo(() => {
    const r = ai.ai?.resolveFeature("triage", ai.settings as never);
    return r?.status === "ok" && r.managed;
  }, [ai.ai, ai.settings]);
  const busy = useRef(false);
  useEffect(() => {
    if (!enabled || managed) return;
    const tick = async () => {
      if (busy.current || document.visibilityState !== "visible") return;
      busy.current = true;
      try {
        const items = await m.claimTriage({ deviceId: deviceId(), limit: 25 });
        if (items.length === 0) return;
        const mod = await loadAi();
        const { text, keys } = mod.renderTriageItems(items);
        let object: unknown = null;
        let model = "unknown";
        for await (const e of ai.run("triage", { items: text })) {
          if (e.event === "meta") model = e.data.model;
          if (e.event === "done") object = e.data.object;
          if (e.event === "error") return;
        }
        if (!object) return;
        const results = mod.triageResults(keys, object as never, model);
        if (results.length) await m.setTriage({ results: results as never });
      } catch {
        /* the lease expires and another device (or the next tick) tries again */
      } finally {
        busy.current = false;
      }
    };
    void tick();
    const t = setInterval(() => void tick(), 60_000);
    return () => clearInterval(t);
  }, [enabled, managed, m, ai]);
  return null;
}

function AiOverlay({ me }: { me: MeView }) {
  useEffect(() => preloadAi(), []);
  return (
    <>
      <NlSearchDialog me={me} />
      <TriageRunner />
    </>
  );
}

export function registerAi(): void {
  registerExtension({
    id: "ai",
    overlay: AiOverlay,
    threadFooter: ThreadAiFooter,
    commands: (ctx) => {
      const id = ctx.openThread?.thread._id;
      return id
        ? {
            "ai.summarize": () => requestThreadAi("summarize", id),
            "ai.instantReply": () => requestThreadAi("instant_reply", id),
          }
        : {};
    },
    paletteItems: (ctx) => {
      const items: PaletteItem[] = [
        {
          id: "ai:search",
          label: "Search with AI",
          section: "AI",
          keywords: ["natural language", "find", "ask"],
          run: () => setNlOpen(true),
        },
        {
          id: "ai:settings",
          label: "AI settings",
          section: "AI",
          keywords: ["provider", "model", "key", "ollama", "local"],
          run: () => navigate("/settings/ai"),
        },
      ];
      const id = ctx.openThread?.thread._id;
      if (id)
        items.push({
          id: "ai:extract",
          label: "Extract actions",
          section: "AI",
          keywords: ["todo", "tasks", "deadlines"],
          run: () => requestThreadAi("extract_actions", id),
        });
      return items;
    },
  });
}

export type { UseAi };
