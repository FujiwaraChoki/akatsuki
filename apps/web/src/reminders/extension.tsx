// Reminders and follow-ups (ARCHITECTURE §10 timers, §4.2 `mail.setReminder` / `cancelReminder`): Shift H opens
// a picker with natural-language times ("fri 3pm", "next week", "nov 3"), "only if nobody replies" (the
// default, `if_no_reply`) or "always", and an optional note (≤ 500 characters, the only text a timer keeps).
// Under an open thread the active reminder shows with Cancel. The AI "Remind me" actions open the same dialog
// with the note filled in.
import { Button, Dialog, errorMessage, Kbd, useToast } from "@akatsuki/ui";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { registerExtension } from "../app/extensions.ts";
import { useBackend } from "../backend/context.tsx";
import type { MeView, ThreadDetail, ThreadId } from "../backend/types.ts";
import { fullDate } from "../lib/format.ts";
import { parseSnoozeInput, snoozePresets } from "../mail/snoozeTimes.ts";

interface ReminderRequest {
  threadId: ThreadId;
  note?: string;
  at?: number;
}

let request: ReminderRequest | null = null;
const listeners = new Set<() => void>();
export function openReminder(r: ReminderRequest | null): void {
  request = r;
  for (const l of [...listeners]) l();
}
const useRequest = () =>
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    () => request,
    () => request,
  );

export const NOTE_MAX = 500;

function ReminderDialog({ me }: { me: MeView }) {
  const req = useRequest();
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const tz = me.profile.timezone;
  const [now, setNow] = useState(() => Date.now());
  const [text, setText] = useState("");
  const [note, setNote] = useState("");
  const [always, setAlways] = useState(false);
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!req) return;
    setNow(Date.now());
    setText(req.at ? "" : "");
    setNote(req.note ?? "");
    setAlways(false);
  }, [req]);
  const presets = useMemo(() => snoozePresets(now, tz).filter((p) => p.id !== "later"), [now, tz]);
  const typed = text.trim() ? parseSnoozeInput(text, now, tz) : (req?.at ?? null);

  const save = async (at: number) => {
    if (!req) return;
    openReminder(null);
    try {
      await m.setReminder({
        threadId: req.threadId,
        at,
        condition: always ? "always" : "if_no_reply",
        ...(note.trim() && { note: note.trim().slice(0, NOTE_MAX) }),
      });
      toast({
        title: always
          ? `Reminder set for ${fullDate(at, tz)}.`
          : `Reminder set for ${fullDate(at, tz)}, if nobody replies.`,
      });
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
    }
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (typed) void save(typed);
    } else if (!text && /^[1-9]$/.test(e.key)) {
      const p = presets[Number(e.key) - 1];
      if (p) {
        e.preventDefault();
        void save(p.at);
      }
    }
  };

  return (
    <Dialog
      open={req !== null}
      onOpenChange={(o) => !o && openReminder(null)}
      title="Remind me"
      size="sm"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        input.current?.focus();
      }}
    >
      <label htmlFor={id} className="ak-field__label">
        When
      </label>
      <input
        id={id}
        ref={input}
        className="ak-input"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="tomorrow 9am, next week, fri 3pm, nov 3"
        autoComplete="off"
        spellCheck={false}
      />
      <p className="ak-picker__hint" aria-live="polite">
        {text.trim()
          ? typed
            ? fullDate(typed, tz)
            : "Not a time we understand yet."
          : req?.at
            ? fullDate(req.at, tz)
            : "Or pick one below."}
      </p>
      <ul className="ak-picker__list">
        {presets.map((p, i) => (
          <li key={p.id}>
            <button type="button" className="ak-picker__item" onClick={() => void save(p.at)}>
              <span>{p.label}</span>
              <span className="ak-picker__when">{fullDate(p.at, tz)}</span>
              <Kbd keys={String(i + 1)} />
            </button>
          </li>
        ))}
      </ul>
      <label className="ak-field__label" htmlFor={`${id}-note`}>
        Note (optional)
      </label>
      <input
        id={`${id}-note`}
        className="ak-input"
        value={note}
        maxLength={NOTE_MAX}
        onChange={(e) => setNote(e.target.value)}
        placeholder="What to do when it comes back"
      />
      <label className="ak-check">
        <input type="checkbox" checked={always} onChange={(e) => setAlways(e.target.checked)} />
        Remind me even if someone replies
      </label>
      {typed && (
        <Button variant="primary" onClick={() => void save(typed)}>
          Set reminder
        </Button>
      )}
    </Dialog>
  );
}

/** Active snooze and reminder of the open thread, with Cancel / Unsnooze. */
function TimersFooter({ thread }: { thread: ThreadDetail }) {
  const backend = useBackend();
  const me = backend.useMe();
  const m = backend.useMutations();
  const { toast } = useToast();
  const tz = me?.profile.timezone ?? "UTC";
  const reminders = thread.timers.filter((t) => t.kind === "reminder" && t.state === "active");
  if (reminders.length === 0) return null;
  return (
    <div className="ak-thread__timers">
      {reminders.map((t) => (
        <div key={t._id} className="ak-thread__timer">
          <span>
            Reminder {fullDate(t.fireAt, tz)}
            {t.condition === "if_no_reply" ? ", if nobody replies" : ""}
            {t.note ? `: ${t.note}` : ""}
          </span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              void m
                .cancelReminder({ timerId: t._id })
                .then(() => toast({ title: "Reminder cancelled." }))
                .catch((e) => toast({ title: errorMessage(e), tone: "error" }))
            }
          >
            Cancel reminder
          </Button>
        </div>
      ))}
    </div>
  );
}

export function registerReminders(): void {
  registerExtension({
    id: "reminders",
    overlay: ReminderDialog,
    threadFooter: TimersFooter,
    commands: (ctx) => {
      const target = (ctx.openThread?.thread._id ?? ctx.targets[0]) as ThreadId | undefined;
      return target && ctx.targets.length <= 1
        ? { "thread.remind": () => openReminder({ threadId: target }) }
        : {};
    },
  });
}
