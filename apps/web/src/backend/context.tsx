import { createContext, type ReactNode, useContext, useMemo } from "react";
import { BodyCache, BodyLoader } from "../mail/bodyCache.ts";
import type { Backend } from "./types.ts";

interface BackendContextValue {
  backend: Backend;
  bodies: BodyLoader;
  syncOrigin: string | null;
}

const Ctx = createContext<BackendContextValue | null>(null);

/** Provides the backend (and its body loader) to the tree, inside the backend's own provider. */
export function BackendProvider({
  backend,
  syncOrigin,
  bodyCache,
  children,
}: {
  backend: Backend;
  syncOrigin: string | null;
  bodyCache?: BodyCache;
  children: ReactNode;
}) {
  const value = useMemo(
    () => ({ backend, syncOrigin, bodies: new BodyLoader(backend.worker, bodyCache ?? new BodyCache()) }),
    [backend, syncOrigin, bodyCache],
  );
  const Inner = backend.Provider;
  return (
    <Ctx.Provider value={value}>
      <Inner>{children}</Inner>
    </Ctx.Provider>
  );
}

function useCtx(): BackendContextValue {
  const v = useContext(Ctx);
  if (!v) throw new Error("BackendProvider is missing.");
  return v;
}

export const useBackend = (): Backend => useCtx().backend;
export const useBodies = (): BodyLoader => useCtx().bodies;
export const useSyncOrigin = (): string | null => useCtx().syncOrigin;
