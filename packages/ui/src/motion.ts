// Reduced motion (SPEC §2.9): every transform goes and the two brand moments are drawn in their final state.
import { useEffect, useState } from "react";

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
