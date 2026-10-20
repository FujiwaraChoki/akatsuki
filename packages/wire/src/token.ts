// Access tokens for the worker and apps/api (ARCHITECTURE §6.2 step 4, §6.4). Web and mobile get a Convex
// JWT (RS256, `aud: "convex"`, 15 minutes) from Better Auth's `authClient.convex.token()`
// (`GET /api/auth/convex/token`), cache it in memory and refresh it 60 s before `exp`. Every worker and apps/api
// request sends `Authorization: Bearer <jwt>`. Tokens are never persisted or logged.
import { WireError } from "./errors.ts";

/** The part of a Better Auth client with the `convexClient()` plugin that we use. */
export interface ConvexTokenClient {
  convex: {
    token(opts?: {
      fetchOptions?: { throw?: boolean };
    }): Promise<{ data?: { token?: string | null } | null }>;
  };
}

/** Anything that mints a fresh JWT: a Better Auth client or a function (desktop, tests). */
export type TokenFetcher = ConvexTokenClient | (() => Promise<string | null>);

export interface AccessTokenOptions {
  /** Refresh this long before `exp`. Default 60 s (desktop uses 120 s). */
  refreshBeforeMs?: number;
  now?: () => number;
}

export interface AccessTokenSource {
  /** A valid JWT, from memory or freshly minted. Throws `WireError("unauthenticated")` when signed out. */
  getAccessToken(opts?: { forceRefresh?: boolean }): Promise<string>;
  /** Forget the cached token (sign-out, or after a 401). */
  clear(): void;
}

function base64UrlDecode(s: string): string {
  const b64 = s
    .replace(/-/g, "+")
    .replace(/_/g, "/")
    .padEnd(Math.ceil(s.length / 4) * 4, "=");
  const bin = atob(b64);
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

/** The `exp` claim of a JWT in ms, without verifying it (the server verifies), or `null` if unreadable. */
export function jwtExpiresAt(jwt: string): number | null {
  const payload = jwt.split(".")[1];
  if (!payload) return null;
  try {
    const claims = JSON.parse(base64UrlDecode(payload)) as { exp?: unknown };
    return typeof claims.exp === "number" ? claims.exp * 1000 : null;
  } catch {
    return null;
  }
}

const SOURCE_BRAND = Symbol.for("akatsuki.wire.AccessTokenSource");

/**
 * Whether `x` is a ready token source rather than a fetcher. Not an `in` check: the Better Auth client is a Proxy
 * that claims every property, so `"getAccessToken" in authClient` is true (D-386, the bug behind web D-269).
 */
function isAccessTokenSource(x: unknown): x is AccessTokenSource {
  if ((typeof x !== "object" && typeof x !== "function") || x === null) return false;
  if ((x as Record<symbol, unknown>)[SOURCE_BRAND] === true) return true;
  const own = (k: string) => Object.hasOwn(x, k) && typeof (x as Record<string, unknown>)[k] === "function";
  return own("getAccessToken") && own("clear");
}

/** Creates a cached token source. Concurrent callers share one in-flight request. */
export function createAccessTokenSource(
  fetcher: TokenFetcher,
  opts: AccessTokenOptions = {},
): AccessTokenSource {
  const refreshBeforeMs = opts.refreshBeforeMs ?? 60_000;
  const now = opts.now ?? (() => Date.now());
  let cached: { token: string; expiresAt: number } | null = null;
  let inflight: Promise<string> | null = null;
  let generation = 0;

  const mint = async (): Promise<string | null> => {
    // A Better Auth client may itself be a callable Proxy, so look for `convex.token` before calling it.
    const client = fetcher as Partial<ConvexTokenClient>;
    if (typeof client.convex?.token === "function") {
      const r = await (fetcher as ConvexTokenClient).convex.token({ fetchOptions: { throw: false } });
      return r.data?.token ?? null;
    }
    if (typeof fetcher === "function") return fetcher();
    throw WireError.of("unauthenticated");
  };

  const source: AccessTokenSource = {
    async getAccessToken({ forceRefresh = false } = {}) {
      if (!forceRefresh && cached && now() < cached.expiresAt - refreshBeforeMs) return cached.token;
      if (!forceRefresh && inflight) return inflight;
      const gen = ++generation;
      const p = (async () => {
        let token: string | null;
        try {
          token = await mint();
        } catch {
          throw WireError.of("unauthenticated");
        }
        if (!token) {
          if (gen === generation) cached = null;
          throw WireError.of("unauthenticated");
        }
        // A token without a readable exp is used once and refetched next time.
        const expiresAt = jwtExpiresAt(token) ?? now() + refreshBeforeMs;
        if (gen === generation) cached = { token, expiresAt };
        return token;
      })();
      inflight = p;
      try {
        return await p;
      } finally {
        if (inflight === p) inflight = null;
      }
    },
    clear() {
      generation++;
      cached = null;
      inflight = null;
    },
  };
  Object.defineProperty(source, SOURCE_BRAND, { value: true });
  return source;
}

let defaultSource: AccessTokenSource | null = null;

/** Configures the process-wide source used by `getAccessToken()` (call once after creating the auth client). */
export function setAccessTokenSource(
  source: AccessTokenSource | TokenFetcher | null,
  opts?: AccessTokenOptions,
) {
  defaultSource =
    source === null ? null : isAccessTokenSource(source) ? source : createAccessTokenSource(source, opts);
}

/** The current user's JWT for the worker and apps/api (§6.2). Configure with `setAccessTokenSource()`. */
export function getAccessToken(opts?: { forceRefresh?: boolean }): Promise<string> {
  if (!defaultSource)
    return Promise.reject(WireError.of("unauthenticated", "No access token source configured."));
  return defaultSource.getAccessToken(opts);
}

/** Forgets the process-wide cached token (sign-out). */
export function clearAccessToken(): void {
  defaultSource?.clear();
}
