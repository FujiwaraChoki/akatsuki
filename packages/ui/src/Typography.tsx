// Typography (First Light, D-482): Instrument Serif headings once per screen at most (subject, empty states,
// sign-in, settings page title), the sentence-case section label, and the mono label only for the reader crumb, the AI
// provenance line and chips.
import type { HTMLAttributes } from "react";
import { cx } from "./config.tsx";

export interface HeadingProps extends HTMLAttributes<HTMLHeadingElement> {
  level?: 1 | 2 | 3;
  /** `subject`: an opened thread's subject; `heading`: app headings; `empty`: empty states; `display`: hero. */
  variant?: "subject" | "heading" | "empty" | "display";
  /** Accent colour (display text, empty states). */
  accent?: boolean;
}

export function Heading({ level = 2, variant = "heading", accent, className, ...rest }: HeadingProps) {
  const Tag = `h${level}` as const;
  return (
    <Tag
      className={cx("ak-heading", `ak-heading--${variant}`, accent && "ak-heading--accent", className)}
      {...rest}
    />
  );
}

export interface EyebrowProps extends HTMLAttributes<HTMLSpanElement> {
  /** `app` (Geist 12/500) or `brand` (Instrument Serif 14). */
  variant?: "app" | "brand";
}

/** A section label in sentence case ("Labels", "Accounts"), 12/500 tertiary. */
export function Eyebrow({ variant = "app", className, ...rest }: EyebrowProps) {
  return <span className={cx("ak-eyebrow", `ak-eyebrow--${variant}`, className)} {...rest} />;
}

/** Mono label (`Inbox · Important`, `Summary · On this device`): the reader crumb and the AI provenance line only. */
export function Crumb({ className, ...rest }: HTMLAttributes<HTMLSpanElement>) {
  return <span className={cx("ak-crumb", className)} {...rest} />;
}
