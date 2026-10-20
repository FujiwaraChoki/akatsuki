// List primitives for dense mail lists (Superhuman-style). A `List` is a listbox; each `ListRow` is an option
// with three independent states: `active` (the keyboard cursor, j/k), `selected` (x / shift+j) and `unread`.
// Rows are plain presentational blocks with fixed heights from the density tokens, so the web app can
// virtualize them. Focus stays on the list (`aria-activedescendant`), never on rows.
import type { HTMLAttributes, ReactNode, Ref } from "react";
import { cx } from "./config.tsx";

export type Density = "compact" | "comfortable";

export interface ListProps extends HTMLAttributes<HTMLDivElement> {
  /** Required: what the list shows ("Inbox", "Search results"). */
  "aria-label": string;
  multiselectable?: boolean;
  /** The id of the active row (`ListRow` `id`), for screen readers. */
  activeId?: string;
  density?: Density;
  ref?: Ref<HTMLDivElement>;
}

export function List({
  multiselectable = true,
  activeId,
  density = "comfortable",
  className,
  tabIndex = 0,
  ref,
  ...rest
}: ListProps) {
  return (
    <div
      ref={ref}
      role="listbox"
      aria-multiselectable={multiselectable}
      aria-activedescendant={activeId}
      tabIndex={tabIndex}
      className={cx("ak-list", `ak-list--${density}`, className)}
      {...rest}
    />
  );
}

export interface ListRowProps extends HTMLAttributes<HTMLDivElement> {
  /** DOM id, referenced by `List activeId`. */
  id: string;
  active?: boolean;
  selected?: boolean;
  unread?: boolean;
  /** Dims the row while an action is pending on it (`pendingOpCount > 0`). */
  pending?: boolean;
  density?: Density;
  ref?: Ref<HTMLDivElement>;
}

export function ListRow({
  active = false,
  selected = false,
  unread = false,
  pending = false,
  density,
  className,
  ref,
  ...rest
}: ListRowProps) {
  return (
    // biome-ignore lint/a11y/useFocusableInteractive: focus stays on the List (aria-activedescendant pattern)
    <div
      ref={ref}
      role="option"
      aria-selected={selected}
      aria-current={active || undefined}
      data-unread={unread || undefined}
      data-pending={pending || undefined}
      className={cx(
        "ak-row",
        density && `ak-row--${density}`,
        active && "ak-row--active",
        selected && "ak-row--selected",
        unread && "ak-row--unread",
        pending && "ak-row--pending",
        className,
      )}
      {...rest}
    />
  );
}

const slot =
  (name: string) =>
  ({ className, ...rest }: HTMLAttributes<HTMLSpanElement> & { children?: ReactNode }) => (
    <span className={cx(`ak-row__${name}`, className)} {...rest} />
  );

/** Leading slot: avatar, checkbox or unread dot. */
export const ListRowLeading = slot("leading");
/** The sender or title; heavier when the row is unread. */
export const ListRowTitle = slot("title");
/** Subject and snippet; truncates. */
export const ListRowSnippet = slot("snippet");
/** Labels, counts, badges. */
export const ListRowMeta = slot("meta");
/** Trailing slot: date or hover actions. */
export const ListRowTrailing = slot("trailing");

/** The unread marker: a 6px ink dot with a text alternative (never red: the red dot is the brand). */
export function UnreadDot({ label = "Unread" }: { label?: string }) {
  return <span className="ak-row__dot" role="img" aria-label={label} />;
}
