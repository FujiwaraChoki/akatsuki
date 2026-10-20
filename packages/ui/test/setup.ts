// jsdom gaps that Radix needs, plus a controllable matchMedia for reduced-motion tests.
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  if (typeof window !== "undefined") cleanup();
  setReducedMotion(false);
});

let reduced = false;
const listeners = new Set<() => void>();

export function setReducedMotion(value: boolean): void {
  reduced = value;
  for (const l of listeners) l();
}

const hasDom = typeof window !== "undefined";

if (hasDom)
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      get matches() {
        return query.includes("prefers-reduced-motion") ? reduced : false;
      },
      media: query,
      onchange: null,
      addEventListener: (_: string, l: () => void) => listeners.add(l),
      removeEventListener: (_: string, l: () => void) => listeners.delete(l),
      addListener: (l: () => void) => listeners.add(l),
      removeListener: (l: () => void) => listeners.delete(l),
      dispatchEvent: () => false,
    }),
  });

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;

if (hasDom) {
  const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.scrollIntoView ??= () => {};
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
}
