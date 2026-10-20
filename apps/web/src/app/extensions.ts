// Slots for part 2 of the web app (compose, settings, AI, billing). Part 1 renders the core mail client and
// leaves these seams so the next features plug in without touching the shell (D-268):
//
// - `routes`: a component for `/compose`, `/settings[/<section>]` or the billing step of onboarding.
// - `commands`: handlers for shortcut ids from packages/shortcuts (e.g. `compose.new`, `thread.reply`,
//   `settings.open`, `ai.summarize`). Only commands with a handler appear in the palette and fire on keys.
// - `paletteItems`: extra palette entries (e.g. "Start your trial").
// - `threadFooter`: rendered under an open thread (reply box, AI summary).
//
// - `overlay`: rendered over the mail shell (the composer, AI dialogs); gets the signed-in user.
// - `keyboardScopes`: replaces the shell's shortcut scopes while it returns a list (the composer returns
//   `["compose"]` while it has focus, so `j`/`e` type text and `ctrl+enter` sends).
//
// Register an extension once at startup: `registerExtension({ id: "compose", ... })` from its module, and
// import that module in `src/main.tsx`. Modules call `notifyExtensions()` when what they return changes.
import type { Scope } from "@akatsuki/shortcuts";
import type { PaletteItem } from "@akatsuki/ui";
import { type ComponentType, useSyncExternalStore } from "react";
import type { MeView, ThreadDetail } from "../backend/types.ts";

export interface CommandContext {
  /** The threads the command applies to (selection, else the open thread, else the cursor row). */
  targets: string[];
  openThread: ThreadDetail | null;
  navigate(to: string): void;
}

export interface AppExtension {
  id: string;
  routes?: {
    compose?: ComponentType;
    settings?: ComponentType<{ section?: string }>;
    /** Onboarding step while `profile.onboarding === "checkout"` (hosted billing, §13.1). */
    checkout?: ComponentType;
  };
  commands?: (ctx: CommandContext) => Partial<Record<string, () => void>>;
  paletteItems?: (ctx: CommandContext) => PaletteItem[];
  threadFooter?: ComponentType<{ thread: ThreadDetail }>;
  overlay?: ComponentType<{ me: MeView }>;
  keyboardScopes?: () => Scope[] | null;
}

const registry: AppExtension[] = [];
const listeners = new Set<() => void>();
let version = 0;

export function registerExtension(ext: AppExtension): void {
  const i = registry.findIndex((e) => e.id === ext.id);
  if (i === -1) registry.push(ext);
  else registry[i] = ext;
  notifyExtensions();
}

/** Tells the shell to re-read commands, palette items and scopes (state inside an extension changed). */
export function notifyExtensions(): void {
  version++;
  for (const l of [...listeners]) l();
}

const subscribe = (cb: () => void) => {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
};

/** Re-renders the caller whenever an extension changes. */
export function useExtensionsVersion(): number {
  return useSyncExternalStore(
    subscribe,
    () => version,
    () => version,
  );
}

/** The first non-null keyboard scope override, if any. */
export function scopeOverride(): Scope[] | null {
  for (const e of registry) {
    const s = e.keyboardScopes?.();
    if (s) return s;
  }
  return null;
}

export function extensions(): readonly AppExtension[] {
  return registry;
}

/** The first registered route component of a kind, if any. */
export function routeSlot<K extends keyof NonNullable<AppExtension["routes"]>>(
  kind: K,
): NonNullable<AppExtension["routes"]>[K] | undefined {
  for (const e of registry) {
    const c = e.routes?.[kind];
    if (c) return c;
  }
  return undefined;
}
