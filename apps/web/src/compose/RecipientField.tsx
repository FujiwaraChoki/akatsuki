// A recipient field: chips for parsed addresses, a text input with contact autocomplete (`contacts.search`,
// §4.2), keyboard-first: type, Up/Down to pick, Enter/Tab/comma to accept, Backspace on empty removes the last
// chip, pasting a list adds every address. Invalid entries stay as text with an inline message.
import { cx } from "@akatsuki/ui";
import { type ClipboardEvent, type KeyboardEvent, type Ref, useId, useState } from "react";
import { useBackend } from "../backend/context.tsx";
import { IconClose } from "../components/Icons.tsx";
import { type Addr, formatAddr, parseAddressList } from "./text.ts";

export interface RecipientFieldProps {
  label: string;
  value: Addr[];
  onChange: (next: Addr[]) => void;
  inputRef?: Ref<HTMLInputElement>;
}

export function RecipientField({ label, value, onChange, inputRef }: RecipientFieldProps) {
  const backend = useBackend();
  const id = useId();
  const [text, setText] = useState("");
  const [active, setActive] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const q = text.trim();
  const contacts = backend.useContacts(q.length >= 1 ? q : null) ?? [];
  const suggestions = contacts
    .filter((c) => !value.some((v) => v.email === c.email.toLowerCase()))
    .slice(0, 6);
  const listId = `${id}-list`;

  const commit = (raw: string): boolean => {
    if (!raw.trim()) return true;
    const { valid, invalid } = parseAddressList(raw);
    const merged = [...value];
    for (const a of valid) if (!merged.some((m) => m.email === a.email)) merged.push(a);
    if (valid.length) onChange(merged);
    if (invalid.length) {
      setText(invalid.join(", "));
      setError(`${invalid[0]} is not an email address.`);
      return false;
    }
    setText("");
    setError(null);
    return true;
  };

  const pick = (c: { email: string; name?: string }) => {
    const a: Addr = c.name
      ? { name: c.name, email: c.email.toLowerCase() }
      : { email: c.email.toLowerCase() };
    if (!value.some((v) => v.email === a.email)) onChange([...value, a]);
    setText("");
    setActive(0);
    setError(null);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const open = suggestions.length > 0 && q.length > 0;
    if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      setActive((i) => (i + (e.key === "ArrowDown" ? 1 : -1) + suggestions.length) % suggestions.length);
      return;
    }
    if (e.key === "Enter" || (e.key === "Tab" && q) || e.key === "," || e.key === ";") {
      if (e.ctrlKey || e.metaKey) return; // ctrl+enter sends
      if (!q) return;
      e.preventDefault();
      if (open && e.key !== "," && e.key !== ";")
        pick(suggestions[Math.min(active, suggestions.length - 1)]!);
      else commit(text);
      return;
    }
    if (e.key === "Backspace" && text === "" && value.length > 0) {
      e.preventDefault();
      onChange(value.slice(0, -1));
    }
  };

  const onPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const pasted = e.clipboardData.getData("text");
    if (/[,;\n]/.test(pasted) || /@/.test(pasted)) {
      e.preventDefault();
      commit(`${text}${pasted.replace(/\n/g, ",")}`);
    }
  };

  const showList = suggestions.length > 0 && q.length > 0;
  return (
    <div className="ak-recipients" data-invalid={error ? true : undefined}>
      <label htmlFor={id} className="ak-compose__label">
        {label}
      </label>
      <div className="ak-recipients__box">
        {value.map((a) => (
          <span key={a.email} className="ak-chip" title={a.email}>
            <span className="ak-chip__text">{a.name ?? a.email}</span>
            <button
              type="button"
              className="ak-chip__remove"
              aria-label={`Remove ${formatAddr(a)}`}
              tabIndex={-1}
              onClick={() => onChange(value.filter((v) => v.email !== a.email))}
            >
              <IconClose />
            </button>
          </span>
        ))}
        <input
          id={id}
          ref={inputRef}
          className="ak-recipients__input"
          value={text}
          autoComplete="off"
          spellCheck={false}
          role="combobox"
          aria-expanded={showList}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={showList ? `${listId}-${active}` : undefined}
          aria-invalid={error ? true : undefined}
          onChange={(e) => {
            setText(e.target.value);
            setActive(0);
            setError(null);
          }}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          onBlur={() => commit(text)}
        />
      </div>
      {showList && (
        <div id={listId} role="listbox" className="ak-recipients__list" aria-label={`${label} suggestions`}>
          {suggestions.map((c, i) => (
            // biome-ignore lint/a11y/useFocusableInteractive: focus stays in the combobox (aria-activedescendant)
            <div
              key={c.email}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className={cx("ak-recipients__option", i === active && "ak-recipients__option--active")}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(c);
              }}
            >
              <span>{c.name ?? c.email}</span>
              {c.name && <span className="ak-recipients__email">{c.email}</span>}
            </div>
          ))}
        </div>
      )}
      {error && (
        <p className="ak-compose__error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
