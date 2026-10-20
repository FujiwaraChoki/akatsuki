// The crimson dawn atmosphere: blurred accent orbs that float and morph slowly (landing, onboarding, empty
// states, the command palette backdrop). Decorative only (`aria-hidden`). With `prefers-reduced-motion` they
// hold still: the stylesheet drops the animation and `data-motion="reduce"` is set for JS-driven layouts.
import { type HTMLAttributes, useEffect, useState } from "react";
import { cx } from "./config.tsx";

const QUERY = "(prefers-reduced-motion: reduce)";

/** `true` when the user asked for reduced motion. Updates live; `false` where matchMedia is missing. */
export function usePrefersReducedMotion(): boolean {
  const get = () => typeof globalThis.matchMedia === "function" && globalThis.matchMedia(QUERY).matches;
  const [reduce, setReduce] = useState(get);
  useEffect(() => {
    if (typeof globalThis.matchMedia !== "function") return;
    const mq = globalThis.matchMedia(QUERY);
    const onChange = () => setReduce(mq.matches);
    onChange();
    mq.addEventListener?.("change", onChange);
    return () => mq.removeEventListener?.("change", onChange);
  }, []);
  return reduce;
}

export interface OrbsProps extends HTMLAttributes<HTMLDivElement> {
  /** `hero`: three large orbs (landing, onboarding). `subtle`: two dimmer ones (empty states, palette). */
  variant?: "hero" | "subtle";
  /** Overrides the user preference (tests, screenshots). */
  reducedMotion?: boolean;
}

export function Orbs({ variant = "subtle", reducedMotion, className, ...rest }: OrbsProps) {
  const prefers = usePrefersReducedMotion();
  const reduce = reducedMotion ?? prefers;
  const count = variant === "hero" ? 3 : 2;
  return (
    <div
      aria-hidden="true"
      data-motion={reduce ? "reduce" : "full"}
      className={cx("ak-orbs", `ak-orbs--${variant}`, className)}
      {...rest}
    >
      {Array.from({ length: count }, (_, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a fixed decorative set
        <span key={i} className={cx("ak-orb", `ak-orb--${i + 1}`)} />
      ))}
    </div>
  );
}
