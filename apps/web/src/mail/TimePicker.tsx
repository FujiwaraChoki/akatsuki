// The snooze (h), remind (shift+h) and send-later picker: presets with number keys, or a typed time ("3d",
// "fri 9am", "2026-11-02 14:00"), resolved in the user's time zone. First Light (SPEC §3.7): an anchored popover
// (280 wide, no scrim) under the row it acts on, with the live parse in mono beside the field.
import { type AnchorRect, Button, Dialog, Kbd } from "@akatsuki/ui";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { fullDate, pickerWhen } from "../lib/format.ts";
import { parseSnoozeInput, snoozePresets } from "./snoozeTimes.ts";

export interface TimePickerProps {
  open: boolean;
  /** `send` is the composer's "Send later" (§10, `kind: "later"`). */
  mode: "snooze" | "remind" | "send";
  count: number;
  timezone: string;
  /** Where the popover hangs (the row, or the composer's Send later button); centred without one. */
  anchor?: AnchorRect | null;
  onOpenChange: (open: boolean) => void;
  onPick: (at: number, returnUnread: boolean) => void;
}

export function TimePicker({ open, mode, count, timezone, anchor, onOpenChange, onPick }: TimePickerProps) {
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
      anchor={
        anchor ?? {
          top: 160,
          bottom: 160,
          left: Math.max(16, (globalThis.innerWidth ?? 1024) / 2 - 140),
          right: 0,
        }
      }
      width={280}
      className="ak-picker"
      onOpenAutoFocus={(e) => {
        e.preventDefault();
        input.current?.focus();
      }}
    >
      <div className="ak-picker__field">
        <label htmlFor={inputId} className="ak-visually-hidden">
          Type a time
        </label>
        <input
          id={inputId}
          ref={input}
          className="ak-picker__input"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="tomorrow 9am, 3d, fri 2pm"
          autoComplete="off"
          spellCheck={false}
          aria-describedby={`${inputId}-hint`}
        />
        <span id={`${inputId}-hint`} className="ak-picker__parse" aria-live="polite">
          {text.trim() ? (typed ? pickerWhen(typed, now, timezone) : "Not a time yet") : ""}
        </span>
      </div>
      <ul className="ak-picker__list">
        {presets.map((p, i) => (
          <li key={p.id}>
            <button
              type="button"
              className="ak-picker__item"
              title={fullDate(p.at, timezone)}
              onClick={() => pick(p.at)}
            >
              <span className="ak-picker__label">{p.label}</span>
              <span className="ak-picker__when">{pickerWhen(p.at, now, timezone)}</span>
              <Kbd keys={String(i + 1)} />
            </button>
          </li>
        ))}
      </ul>
      {mode === "snooze" && (
        <label className="ak-picker__check">
          <input type="checkbox" checked={returnUnread} onChange={(e) => setReturnUnread(e.target.checked)} />
          Bring back as unread
        </label>
      )}
      {typed && (
        <div className="ak-picker__go">
          <Button size="sm" variant="ink" onClick={() => pick(typed)}>
            {mode === "snooze" ? "Snooze" : mode === "send" ? "Schedule" : "Set reminder"}
          </Button>
          <span className="ak-picker__when">{fullDate(typed, timezone)}</span>
        </div>
      )}
    </Dialog>
  );
}
