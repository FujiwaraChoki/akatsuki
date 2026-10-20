// The snooze (h) and remind (shift+h) picker: presets with number keys, or a typed time ("3d", "fri 9am",
// "2026-11-02 14:00"), resolved in the user's time zone.
import { Button, Dialog, Kbd } from "@akatsuki/ui";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { fullDate } from "../lib/format.ts";
import { parseSnoozeInput, snoozePresets } from "./snoozeTimes.ts";

export interface TimePickerProps {
  open: boolean;
  /** `send` is the composer's "Send later" (§10, `kind: "later"`). */
  mode: "snooze" | "remind" | "send";
  count: number;
  timezone: string;
  onOpenChange: (open: boolean) => void;
  onPick: (at: number, returnUnread: boolean) => void;
}

export function TimePicker({ open, mode, count, timezone, onOpenChange, onPick }: TimePickerProps) {
  const [now, setNow] = useState(() => Date.now());
  const [text, setText] = useState("");
  const [returnUnread, setReturnUnread] = useState(false);
  const inputId = useId();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (open) {
      setNow(Date.now());
      setText("");
    }
  }, [open]);
  const presets = useMemo(() => snoozePresets(now, timezone), [now, timezone]);
  const typed = text.trim() ? parseSnoozeInput(text, now, timezone) : null;

  const pick = (at: number) => {
    onPick(at, returnUnread);
    onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (typed) pick(typed);
      else if (!text.trim() && presets[0]) pick(presets[0].at);
    } else if (!text && /^[1-9]$/.test(e.key)) {
      const p = presets[Number(e.key) - 1];
      if (p) {
        e.preventDefault();
        pick(p.at);
      }
    }
  };

  const title =
    mode === "snooze"
      ? count > 1
        ? `Snooze ${count} conversations`
        : "Snooze until"
      : mode === "send"
        ? "Send later"
        : "Remind me if no reply by";
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      size="sm"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        input.current?.focus();
      }}
    >
      <label htmlFor={inputId} className="ak-field__label">
        Type a time
      </label>
      <input
        id={inputId}
        ref={input}
        className="ak-input"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        placeholder="tomorrow 9am, 3d, fri 2pm"
        autoComplete="off"
        spellCheck={false}
        aria-describedby={`${inputId}-hint`}
      />
      <p id={`${inputId}-hint`} className="ak-picker__hint" aria-live="polite">
        {text.trim()
          ? typed
            ? fullDate(typed, timezone)
            : "Not a time we understand yet."
          : "Or pick one below."}
      </p>
      <ul className="ak-picker__list">
        {presets.map((p, i) => (
          <li key={p.id}>
            <button type="button" className="ak-picker__item" onClick={() => pick(p.at)}>
              <span>{p.label}</span>
              <span className="ak-picker__when">{fullDate(p.at, timezone)}</span>
              <Kbd keys={String(i + 1)} />
            </button>
          </li>
        ))}
      </ul>
      {mode === "snooze" && (
        <label className="ak-check">
          <input type="checkbox" checked={returnUnread} onChange={(e) => setReturnUnread(e.target.checked)} />
          Bring back as unread
        </label>
      )}
      {typed && (
        <Button variant="primary" onClick={() => pick(typed)}>
          {mode === "snooze" ? "Snooze" : mode === "send" ? "Schedule" : "Set reminder"}
        </Button>
      )}
    </Dialog>
  );
}
