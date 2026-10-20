// Badges: unread counts (capped like `mail.unreadCounts`, which stops at 100) as mono grey numbers, and labels as
// mono caps chips (First Light: no red counts, no filled pills).
import type { HTMLAttributes } from "react";
import { cx } from "./config.tsx";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: "accent" | "neutral" | "outline";
  /** A count; above `max` it shows "99+". Zero renders nothing. */
  count?: number;
  max?: number;
}

/** "99+" above `max`, else the number. */
export function formatCount(count: number, max = 99): string {
  return count > max ? `${max}+` : String(Math.max(0, Math.floor(count)));
}

export function Badge({ tone = "neutral", count, max = 99, className, children, ...rest }: BadgeProps) {
  if (count !== undefined && count <= 0 && children === undefined) return null;
  const text = count !== undefined ? formatCount(count, max) : children;
  return (
    <span
      className={cx("ak-badge", `ak-badge--${tone}`, count === undefined && "ak-badge--label", className)}
      {...rest}
    >
      {text}
    </span>
  );
}
