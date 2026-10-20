import type { HTMLAttributes } from "react";
import { cx } from "./config.tsx";

export interface SpinnerProps extends HTMLAttributes<HTMLSpanElement> {
  /** Announced to screen readers. */
  label?: string;
  size?: "sm" | "md" | "lg";
}

/** A crimson ring. With reduced motion it pulses its opacity instead of spinning. */
export function Spinner({ label = "Loading", size = "md", className, ...rest }: SpinnerProps) {
  return (
    <span
      role="status"
      aria-label={label}
      className={cx("ak-spinner", `ak-spinner--${size}`, className)}
      {...rest}
    >
      <span className="ak-spinner__ring" aria-hidden="true" />
    </span>
  );
}
