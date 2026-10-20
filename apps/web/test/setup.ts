// jsdom gaps that Radix and the app need: matchMedia (reduced motion, colour scheme), ResizeObserver,
// scrollIntoView, pointer capture. Storage and the URL are reset between tests.
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach } from "vitest";

const hasDom = typeof window !== "undefined";

beforeEach(() => {
  if (!hasDom) return;
  localStorage.clear();
  window.history.replaceState(null, "", "/");
  document.documentElement.removeAttribute("data-theme");
});

afterEach(() => {
  if (hasDom) cleanup();
});

if (hasDom) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }),
  });
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;
  const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.scrollIntoView ??= () => {};
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  // Fixed geometry so the virtual list renders a window of rows.
  Object.defineProperty(window.HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 800 });
  Object.defineProperty(window.HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 600 });
}
