// A small filterable list in a dialog: label picker (l), "Go to label" (g l), "Move to folder" (v).
// Keyboard: type to filter; the palette.* bindings of packages/shortcuts move, choose and close.
import { Dialog, eventMatchesShortcut, useUiConfig } from "@akatsuki/ui";
import { type KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";

export interface PickerItem {
  id: string;
  label: string;
  detail?: string;
  /** Shows a check (label currently applied). */
  checked?: boolean;
}

export interface ListPickerProps {
  open: boolean;
  title: string;
  placeholder: string;
  items: PickerItem[];
  emptyText: string;
  onOpenChange: (open: boolean) => void;
  onPick: (item: PickerItem) => void;
  /** Offers "Create label “<query>”" when the typed name matches no item exactly (labels.create). */
  onCreate?: (name: string) => void;
  /** The noun in that row ("label"). */
  createNoun?: string;
}

/** The id of the "Create …" row. */
export const CREATE_ID = "\u0000create";

export function ListPicker({
  open,
  title,
  placeholder,
  items,
  emptyText,
  onOpenChange,
  onPick,
  onCreate,
  createNoun = "label",
}: ListPickerProps) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const listId = useId();
  const config = useUiConfig();
  useEffect(() => {
    if (open) {
      setQ("");
      setActive(0);
    }
  }, [open]);
  const shown = useMemo(() => {
    const s = q.trim().toLowerCase();
    const matches = s ? items.filter((i) => `${i.label} ${i.detail ?? ""}`.toLowerCase().includes(s)) : items;
    const exact = items.some((i) => i.label.toLowerCase() === s);
    return onCreate && s && !exact
      ? [...matches, { id: CREATE_ID, label: `Create ${createNoun} “${q.trim()}”` }]
      : matches;
  }, [items, q, onCreate, createNoun]);
  const current = shown[Math.min(active, shown.length - 1)];

  const choose = (item: PickerItem | undefined) => {
    if (!item) return;
    if (item.id === CREATE_ID) onCreate?.(q.trim());
    else onPick(item);
    onOpenChange(false);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const ev = e.nativeEvent;
    if (eventMatchesShortcut(ev, "palette.next", config)) {
      e.preventDefault();
      setActive((a) => (shown.length ? (a + 1) % shown.length : 0));
    } else if (eventMatchesShortcut(ev, "palette.prev", config)) {
      e.preventDefault();
      setActive((a) => (shown.length ? (a - 1 + shown.length) % shown.length : 0));
    } else if (eventMatchesShortcut(ev, "palette.run", config)) {
      e.preventDefault();
      choose(current);
    }
  };

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
      <input
        ref={input}
        className="ak-input"
        role="combobox"
        aria-expanded="true"
        aria-controls={listId}
        aria-label={title}
        aria-activedescendant={current ? `${listId}-${current.id}` : undefined}
        placeholder={placeholder}
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          setActive(0);
        }}
        onKeyDown={onKeyDown}
        autoComplete="off"
      />
      <div id={listId} role="listbox" aria-label={title} className="ak-picker__list">
        {shown.length === 0 && <p className="ak-picker__hint">{emptyText}</p>}
        {shown.map((item) => (
          // biome-ignore lint/a11y/useFocusableInteractive: focus stays in the combobox (aria-activedescendant)
          // biome-ignore lint/a11y/useKeyWithClickEvents: the combobox handles keys
          <div
            key={item.id}
            id={`${listId}-${item.id}`}
            role="option"
            aria-selected={item === current}
            aria-checked={item.checked}
            className="ak-picker__item"
            data-active={item === current || undefined}
            onClick={() => choose(item)}
            onPointerMove={() => setActive(shown.indexOf(item))}
          >
            <span>{item.label}</span>
            {item.detail && <span className="ak-picker__when">{item.detail}</span>}
            {item.checked && <span className="ak-picker__check">Applied</span>}
          </div>
        ))}
      </div>
    </Dialog>
  );
}
