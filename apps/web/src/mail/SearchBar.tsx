// The search field (§11): one grammar (parseQuery from @akatsuki/mail-model), operator and value suggestions
// as you type, parse errors shown inline once the user pauses (never for an operator still waiting for its value)
// or presses Enter. `search.submit` runs it, `search.close` leaves.
import { parseQuery } from "@akatsuki/mail-model";
import { eventMatchesShortcut, useUiConfig } from "@akatsuki/ui";
import { type KeyboardEvent, type Ref, useEffect, useId, useMemo, useState } from "react";
import { IconSearch } from "../components/Icons.tsx";
import { applySuggestion, lastToken, type SuggestContext, suggest } from "./searchSuggest.ts";

/** How long typing pauses before a parse error shows. */
export const ERROR_PAUSE_MS = 900;

export interface SearchBarProps {
  value: string;
  onChange: (v: string) => void;
  onSubmit: (q: string) => void;
  onClose: () => void;
  onFocusChange: (focused: boolean) => void;
  inputRef: Ref<HTMLInputElement>;
  context: SuggestContext;
  /** The `from:`/`to:` value being typed, for contact lookups (`contacts.search`). */
  onContactQuery: (q: string | null) => void;
  timezone: string;
}

export function SearchBar(props: SearchBarProps) {
  const { value, onChange, onSubmit, onClose, context, timezone } = props;
  const config = useUiConfig();
  const [focused, setFocused] = useState(false);
  const [active, setActive] = useState(-1);
  const listId = useId();
  const items = useMemo(() => (focused ? suggest(value, context) : []), [focused, value, context]);
  const errors = useMemo(
    () => (value.trim() ? parseQuery(value, { nowAt: Date.now(), timezone }).errors : []),
    [value, timezone],
  );

  // Errors wait for a pause: "from:" is not an error while the value is being typed.
  const [showErrors, setShowErrors] = useState(false);
  useEffect(() => {
    setShowErrors(false);
    if (errors.length === 0 || /^-?[a-z]+:$/i.test(lastToken(value).token)) return;
    const t = setTimeout(() => setShowErrors(true), ERROR_PAUSE_MS);
    return () => clearTimeout(t);
  }, [value, errors.length]);

  const change = (v: string) => {
    onChange(v);
    setActive(-1);
    const { token } = lastToken(v);
    const m = /^-?(from|to|cc):"?(.*)$/i.exec(token);
    props.onContactQuery(m ? (m[2] ?? "") : null);
  };

  const pick = (i: number) => {
    const s = items[i];
    if (s) change(applySuggestion(value, s));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const ev = e.nativeEvent;
    if (items.length > 0 && eventMatchesShortcut(ev, "palette.next", config)) {
      e.preventDefault();
      setActive((a) => (a + 1) % items.length);
    } else if (items.length > 0 && eventMatchesShortcut(ev, "palette.prev", config)) {
      e.preventDefault();
      setActive((a) => (a <= 0 ? items.length - 1 : a - 1));
    } else if (e.key === "Tab" && active >= 0) {
      e.preventDefault();
      pick(active);
    } else if (eventMatchesShortcut(ev, "search.submit", config)) {
      e.preventDefault();
      if (active >= 0) pick(active);
      else if (value.trim()) {
        if (errors.length > 0) setShowErrors(true);
        onSubmit(value.trim());
      }
    } else if (eventMatchesShortcut(ev, "search.close", config)) {
      e.preventDefault();
      if (active >= 0) setActive(-1);
      else onClose();
    }
  };

  return (
    <div className="ak-search" data-focused={focused || undefined}>
      <span className="ak-search__icon" aria-hidden="true">
        <IconSearch />
      </span>
      <input
        ref={props.inputRef}
        className="ak-search__input"
        type="search"
        role="combobox"
        aria-label="Search mail"
        aria-expanded={items.length > 0}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={active >= 0 ? `${listId}-${active}` : undefined}
        aria-invalid={(showErrors && errors.length > 0) || undefined}
        placeholder="Search (from:, is:unread, has:attachment, before:2026-01-01)"
        value={value}
        onChange={(e) => change(e.target.value)}
        onKeyDown={onKeyDown}
        onFocus={() => {
          setFocused(true);
          props.onFocusChange(true);
        }}
        onBlur={() => {
          setFocused(false);
          setActive(-1);
          props.onFocusChange(false);
        }}
        autoComplete="off"
        spellCheck={false}
      />
      {items.length > 0 && (
        <div id={listId} role="listbox" aria-label="Suggestions" className="ak-search__suggest">
          {items.map((s, i) => (
            // biome-ignore lint/a11y/useFocusableInteractive: focus stays in the combobox (aria-activedescendant)
            <div
              key={s.insert}
              id={`${listId}-${i}`}
              role="option"
              aria-selected={i === active}
              className="ak-search__option"
              data-active={i === active || undefined}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(i);
              }}
            >
              <span className="ak-search__op">{s.label}</span>
              {s.hint && <span className="ak-search__hint">{s.hint}</span>}
            </div>
          ))}
        </div>
      )}
      {focused && showErrors && errors.length > 0 && (
        <p className="ak-search__error" role="status">
          {errors[0]}
        </p>
      )}
    </div>
  );
}
