// Hardware keyboard shortcuts (iPad first). Screens register handlers for a scope while they are focused; the
// provider feeds every key press to the shared matcher (packages/shortcuts) with the scopes that are active now
// and runs the newest handler for the matched id.
import type { Keymap, Scope } from "@akatsuki/shortcuts";
import { type KeyPressEvent, useKeyEventListener } from "expo-key-event";
import { useFocusEffect } from "expo-router";
import { createContext, type ReactNode, useCallback, useContext, useEffect, useMemo, useRef } from "react";
import { Platform } from "react-native";
import { createMobileMatcher, keyPlatform, toKeyEventLike } from "../vm/keyboard";

export type Handlers = Partial<Record<string, () => void>>;

interface Layer {
  id: number;
  scope: Scope;
  handlers: { current: Handlers };
}

interface KeyboardApi {
  register(scope: Scope, handlers: { current: Handlers }): () => void;
  /** Scopes of the focused screen (for the palette), newest first. */
  activeScopes(): Scope[];
  /** Runs a command id as if its shortcut was pressed (the palette uses this). */
  run(id: string): boolean;
  platform: ReturnType<typeof keyPlatform>;
  keymap: Keymap;
}

const Ctx = createContext<KeyboardApi | null>(null);

export function KeyboardProvider({ children, keymap }: { children: ReactNode; keymap: Keymap }) {
  const platform = keyPlatform(
    Platform.OS,
    typeof navigator !== "undefined" ? navigator.userAgent : undefined,
  );
  const layers = useRef<Layer[]>([]);
  const nextId = useRef(1);
  const matcher = useMemo(() => createMobileMatcher(platform, keymap), [platform, keymap]);

  const run = useCallback((id: string): boolean => {
    for (let i = layers.current.length - 1; i >= 0; i--) {
      const handler = layers.current[i]!.handlers.current[id];
      if (handler) {
        handler();
        return true;
      }
    }
    return false;
  }, []);

  const onKey = useCallback(
    (event: { eventType: string } & Parameters<typeof toKeyEventLike>[0]) => {
      if (event.eventType !== "press") return;
      const active = new Set<Scope>(layers.current.map((l) => l.scope));
      const result = matcher.handle(toKeyEventLike(event), active);
      if (result.kind === "match") run(result.id);
    },
    [matcher, run],
  );

  useKeyEventListener((e) => onKey(e as KeyPressEvent), { captureModifiers: true, preventReload: true });

  const api = useMemo<KeyboardApi>(
    () => ({
      platform,
      keymap,
      run,
      activeScopes: () => [...new Set(layers.current.map((l) => l.scope).reverse())],
      register(scope, handlers) {
        const layer = { id: nextId.current++, scope, handlers };
        layers.current = [...layers.current, layer];
        matcher.reset();
        return () => {
          layers.current = layers.current.filter((l) => l.id !== layer.id);
        };
      },
    }),
    [platform, keymap, matcher, run],
  );

  return <Ctx.Provider value={api}>{children}</Ctx.Provider>;
}

export function useKeyboard(): KeyboardApi {
  const api = useContext(Ctx);
  if (!api) throw new Error("useKeyboard outside KeyboardProvider");
  return api;
}

/** Registers shortcut handlers for `scope` while the screen is focused. */
export function useShortcuts(scope: Scope, handlers: Handlers): void {
  const api = useKeyboard();
  const ref = useRef(handlers);
  ref.current = handlers;
  useFocusEffect(useCallback(() => api.register(scope, ref), [api, scope]));
}

/** Registers handlers for the lifetime of a component (the app shell's global shortcuts). */
export function useGlobalShortcuts(handlers: Handlers): void {
  const api = useKeyboard();
  const ref = useRef(handlers);
  ref.current = handlers;
  useEffect(() => api.register("global", ref), [api]);
}
