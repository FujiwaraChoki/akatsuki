// Public build-time configuration (ARCHITECTURE §18 "Clients"): EXPO_PUBLIC_CONVEX_URL, EXPO_PUBLIC_CONVEX_SITE_URL,
// EXPO_PUBLIC_SYNC_URL, EXPO_PUBLIC_API_URL, plus EXPO_PUBLIC_LOGIN_PROVIDERS (D-332). Values are public: they are
// compiled into the app. http is accepted only for local development hosts.

export type SocialProvider = "google" | "microsoft";

export interface AppEnv {
  convexUrl: string;
  convexSiteUrl: string;
  syncUrl: string;
  apiUrl: string;
  /** Social login buttons to show (the server enables a provider only when its OAuth client is configured). */
  loginProviders: SocialProvider[];
}

export interface RawEnv {
  EXPO_PUBLIC_CONVEX_URL?: string;
  EXPO_PUBLIC_CONVEX_SITE_URL?: string;
  EXPO_PUBLIC_SYNC_URL?: string;
  EXPO_PUBLIC_API_URL?: string;
  EXPO_PUBLIC_LOGIN_PROVIDERS?: string;
}

const LOCAL_DEV =
  /^(localhost|127(?:\.\d{1,3}){3}|10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(1[6-9]|2\d|3[01])(?:\.\d{1,3}){2}|[a-z0-9-]+\.local)$/i;

function url(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const u = new URL(value.trim());
    if (u.username || u.password || u.search || u.hash) return null;
    if (u.protocol === "https:" || (u.protocol === "http:" && LOCAL_DEV.test(u.hostname)))
      return u.toString().replace(/\/+$/, "");
    return null;
  } catch {
    return null;
  }
}

export function parseLoginProviders(value: string | undefined): SocialProvider[] {
  const out: SocialProvider[] = [];
  for (const p of (value ?? "").split(",").map((s) => s.trim().toLowerCase())) {
    if ((p === "google" || p === "microsoft") && !out.includes(p)) out.push(p);
  }
  return out;
}

/** Validates the raw variables. `missing` names every variable that is absent or not an acceptable URL. */
export function readEnv(raw: RawEnv): { env: AppEnv | null; missing: string[] } {
  const convexUrl = url(raw.EXPO_PUBLIC_CONVEX_URL);
  const convexSiteUrl = url(raw.EXPO_PUBLIC_CONVEX_SITE_URL);
  const syncUrl = url(raw.EXPO_PUBLIC_SYNC_URL);
  const apiUrl = url(raw.EXPO_PUBLIC_API_URL);
  const missing: string[] = [];
  if (!convexUrl) missing.push("EXPO_PUBLIC_CONVEX_URL");
  if (!convexSiteUrl) missing.push("EXPO_PUBLIC_CONVEX_SITE_URL");
  if (!syncUrl) missing.push("EXPO_PUBLIC_SYNC_URL");
  if (!apiUrl) missing.push("EXPO_PUBLIC_API_URL");
  if (!convexUrl || !convexSiteUrl || !syncUrl || !apiUrl) return { env: null, missing };
  return {
    env: {
      convexUrl,
      convexSiteUrl,
      syncUrl,
      apiUrl,
      loginProviders: parseLoginProviders(raw.EXPO_PUBLIC_LOGIN_PROVIDERS),
    },
    missing,
  };
}
