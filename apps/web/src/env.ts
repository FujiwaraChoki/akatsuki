// Public build-time configuration (ARCHITECTURE §18 "Clients"; D-260). Only `VITE_*` names reach the bundle.
// In development the Convex URLs default to the local harness ports from docs/DEV.md, so no env file is
// needed. A production build without them shows a configuration screen instead of failing silently.

declare const __AKATSUKI_MOCK__: boolean;

export type SocialProvider = "google" | "microsoft";

export interface AppEnv {
  backend: "convex" | "mock";
  convexUrl: string | null;
  /** `*.convex.site`: Better Auth lives at `/api/auth/*`, and it is the JWT issuer. */
  convexSiteUrl: string | null;
  /** Sync worker base URL (bodies, connect, provider search). */
  syncUrl: string | null;
  apiUrl: string | null;
  /** Social login buttons to show (D-261). The server decides which providers are configured. */
  loginProviders: SocialProvider[];
}

const LOCAL_CONVEX_URL = "http://127.0.0.1:3410";
const LOCAL_CONVEX_SITE_URL = "http://127.0.0.1:3411";
const LOCAL_SYNC_URL = "http://127.0.0.1:8787";

function clean(v: string | undefined): string | null {
  const s = v?.trim();
  return s ? s.replace(/\/+$/, "") : null;
}

export function parseLoginProviders(v: string | undefined): SocialProvider[] {
  const out: SocialProvider[] = [];
  for (const p of (v ?? "").split(",").map((s) => s.trim().toLowerCase())) {
    if ((p === "google" || p === "microsoft") && !out.includes(p)) out.push(p);
  }
  return out;
}

export function readEnv(raw: Record<string, string | boolean | undefined> = import.meta.env): AppEnv {
  const s = (k: string) => (typeof raw[k] === "string" ? (raw[k] as string) : undefined);
  const dev = raw.DEV === true;
  const mock =
    __AKATSUKI_MOCK__ &&
    (s("MODE") === "mock" || s("MODE") === "test" || s("VITE_AKATSUKI_BACKEND") === "mock");
  return {
    backend: mock ? "mock" : "convex",
    convexUrl: clean(s("VITE_CONVEX_URL")) ?? (dev ? LOCAL_CONVEX_URL : null),
    convexSiteUrl: clean(s("VITE_CONVEX_SITE_URL")) ?? (dev ? LOCAL_CONVEX_SITE_URL : null),
    syncUrl: clean(s("VITE_SYNC_URL")) ?? (dev ? LOCAL_SYNC_URL : null),
    apiUrl: clean(s("VITE_API_URL")),
    loginProviders: parseLoginProviders(s("VITE_LOGIN_PROVIDERS")),
  };
}

/** The sync worker origin, for the body iframe CSP and image URL checks (§14.3, §14.4). */
export function originOf(url: string | null): string | null {
  if (!url) return null;
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
}
