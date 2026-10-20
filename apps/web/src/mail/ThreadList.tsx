// The virtualized thread list (ARCHITECTURE §16.2). Fixed row heights from the density tokens, so only the
// visible window is rendered and nothing shifts as pages stream in. Focus stays on the list
// (`aria-activedescendant`); the keyboard cursor (j/k) and the selection (x) are owned by the caller.
import { SYS } from "@akatsuki/mail-model";
import { layout } from "@akatsuki/tokens";
import {
  Badge,
  List,
  ListRow,
  ListRowLeading,
  ListRowMeta,
  ListRowSnippet,
  ListRowTitle,
  ListRowTrailing,
  Spinner,
  UnreadDot,
} from "@akatsuki/ui";
import { memo, type ReactNode, type Ref, useEffect, useLayoutEffect, useRef, useState } from "react";
import { TriageChip } from "../ai/TriageChip.tsx";
import type { LabelView, ListStatus, ThreadRow } from "../backend/types.ts";
import { IconPaperclip, IconStar } from "../components/Icons.tsx";
import { listDate, sendersLabel } from "../lib/format.ts";
import type { Density } from "../lib/theme.ts";
import { nearEnd, scrollToReveal, windowFor } from "./virtual.ts";

export const ROW_HEIGHT: Record<Density, number> = {
  compact: layout.rowHeightCompact,
  comfortable: layout.rowHeightComfortable,
};

/** Below this width a row has two lines (sender and time, then subject and preview); keep in step with app.css. */
export const TWO_LINE_QUERY = "(max-width: 600px)";
/** Height of a two-line row (the `--ak-row-height` app.css sets for the same query). */
export const TWO_LINE_ROW_HEIGHT = 64;

const twoLineQuery = () =>
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(TWO_LINE_QUERY)
    : null;

function useTwoLineRows(): boolean {
  const query = twoLineQuery;
  const [on, setOn] = useState(() => query()?.matches ?? false);
  useEffect(() => {
    const mq = query();
    if (!mq) return;
    const update = () => setOn(mq.matches);
    update();
    mq.addEventListener("change", update);
    return () => mq.removeEventListener("change", update);
  }, []);
  return on;
}

export interface ThreadListProps {
  rows: readonly ThreadRow[];
  status: ListStatus;
  loadMore: (n: number) => void;
  cursorId: string | null;
  openId: string | null;
  selected: ReadonlySet<string>;
  density: Density;
  timezone: string;
  now: number;
  myEmails: ReadonlySet<string>;
  labelsByKey: ReadonlyMap<string, LabelView>;
  /** Show the snooze time instead of the date (Snoozed view). */
  showSnoozedUntil?: boolean;
  ariaLabel: string;
  empty: ReactNode;
  onActivate: (id: string) => void;
  onToggleSelect: (id: string) => void;
  listRef?: Ref<HTMLDivElement>;
}

export const rowDomId = (id: string) => `row-${id}`;

export function ThreadList(props: ThreadListProps) {
  const { rows, status, loadMore, cursorId, density } = props;
  const twoLine = useTwoLineRows();
  const h = twoLine ? TWO_LINE_ROW_HEIGHT : ROW_HEIGHT[density];
  const scroller = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(800);
  const frame = useRef(0);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    setViewport(el.clientHeight || 800);
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => setViewport(el.clientHeight || 800));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const win = windowFor(rows.length, h, scrollTop, viewport);

  // Infinite scroll: ask for the next page before the user reaches the end.
  useEffect(() => {
    if (status === "more" && nearEnd(win, rows.length)) loadMore(50);
  }, [status, win.end, rows.length, loadMore, win]);

  // Keep the keyboard cursor visible.
  const cursorIndex = cursorId ? rows.findIndex((r) => r._id === cursorId) : -1;
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || cursorIndex < 0) return;
    const next = scrollToReveal(cursorIndex, h, el.scrollTop, el.clientHeight || viewport);
    if (next !== null) {
      el.scrollTop = next;
      setScrollTop(next);
    }
  }, [cursorIndex, h, viewport]);

  const onScroll = () => {
    if (frame.current) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      setScrollTop(scroller.current?.scrollTop ?? 0);
    });
  };

  if (status === "loading" && rows.length === 0)
    return (
      <div className="ak-listpane__loading">
        <Spinner label="Loading conversations" />
      </div>
    );
  // Focusable (not tabbable) so focus can land here after moving to an empty mailbox, never on <body>.
  if (rows.length === 0 && status === "done")
    return (
      <div key="empty" className="ak-listpane__empty" tabIndex={-1}>
        {props.empty}
      </div>
    );

  const visible = rows.slice(win.start, win.end);
  return (
    <div key="list" className="ak-scroller" ref={scroller} onScroll={onScroll}>
      <List
        ref={props.listRef}
        aria-label={props.ariaLabel}
        activeId={cursorId ? rowDomId(cursorId) : undefined}
        density={density}
        className="ak-threadlist"
        data-ak-tab-scope=""
        style={{ height: win.height }}
      >
        {visible.map((row, i) => (
          <Row
            key={row._id}
            row={row}
            top={(win.start + i) * h}
            active={row._id === cursorId}
            open={row._id === props.openId}
            selected={props.selected.has(row._id)}
            density={density}
            timezone={props.timezone}
            now={props.now}
            myEmails={props.myEmails}
            labelsByKey={props.labelsByKey}
            showSnoozedUntil={props.showSnoozedUntil}
            onActivate={props.onActivate}
            onToggleSelect={props.onToggleSelect}
          />
        ))}
      </List>
      {status === "loading_more" && (
        <div className="ak-listpane__more">
          <Spinner size="sm" label="Loading more" />
        </div>
      )}
    </div>
  );
}

interface RowProps {
  row: ThreadRow;
  top: number;
  active: boolean;
  open: boolean;
  selected: boolean;
  density: Density;
  timezone: string;
  now: number;
  myEmails: ReadonlySet<string>;
  labelsByKey: ReadonlyMap<string, LabelView>;
  showSnoozedUntil?: boolean;
  onActivate: (id: string) => void;
  onToggleSelect: (id: string) => void;
}

const Row = memo(function Row({
  row,
  top,
  active,
  open,
  selected,
  density,
  timezone,
  now,
  myEmails,
  labelsByKey,
  showSnoozedUntil,
  onActivate,
  onToggleSelect,
}: RowProps) {
  const unread = row.labelKeys.includes(SYS.unread);
  const starred = row.labelKeys.includes(SYS.starred);
  const userLabels = row.labelKeys
    .filter((k) => k.startsWith("l:"))
    .map((k) => labelsByKey.get(k))
    .filter(
      (l): l is LabelView => l !== undefined && !l.hidden && l.role === undefined && l.kind !== "folder",
    )
    .slice(0, 2);
  const when = showSnoozedUntil && row.snoozedUntil ? row.snoozedUntil : row.sortAt;
  return (
    <ListRow
      id={rowDomId(row._id)}
      active={active}
      selected={selected}
      unread={unread}
      pending={row.pendingOpCount > 0}
      density={density}
      className="ak-threadrow"
      data-open={open || undefined}
      data-thread-id={row._id}
      style={{ transform: `translateY(${top}px)` }}
      onClick={(e) => {
        if (e.metaKey || e.ctrlKey || e.shiftKey) onToggleSelect(row._id);
        else onActivate(row._id);
      }}
    >
      <ListRowLeading className="ak-threadrow__lead">
        {selected ? (
          <span className="ak-threadrow__check" aria-hidden="true" />
        ) : unread ? (
          <UnreadDot />
        ) : null}
      </ListRowLeading>
      <ListRowTitle>
        {sendersLabel(row, myEmails)}
        {row.messageCount > 1 && <span className="ak-threadrow__count">{row.messageCount}</span>}
      </ListRowTitle>
      <ListRowSnippet>
        <span className="ak-threadrow__subject">{row.subject || "(no subject)"}</span>
        {row.snippet && <span className="ak-threadrow__preview">{row.snippet}</span>}
      </ListRowSnippet>
      <ListRowMeta>
        <TriageChip triage={row.triage} />
        {userLabels.map((l) => (
          <Badge key={l._id} tone="outline">
            {l.name}
          </Badge>
        ))}
        {row.hasAttachments && (
          <>
            <IconPaperclip />
            <span className="ak-visually-hidden">Has attachments</span>
          </>
        )}
        {starred && (
          <>
            <IconStar filled className="ak-threadrow__star" />
            <span className="ak-visually-hidden">Starred</span>
          </>
        )}
      </ListRowMeta>
      <ListRowTrailing>
        <time dateTime={new Date(when).toISOString()}>{listDate(when, now, timezone)}</time>
      </ListRowTrailing>
    </ListRow>
  );
});
