// Mobile sign-in handoff (ARCHITECTURE §6.4, D-408), the pure parts: the session never travels in a custom-scheme
// URL. Sign-in starts with `callbackURL` on the deployment's own https origin carrying a PKCE-style challenge; the
// app receives only a single-use code at `akatsuki://auth/callback` and redeems it with the verifier it kept.

/** Where the app is sent with `?code=` (or `?error=`). */
export const HANDOFF_CALLBACK = "akatsuki://auth/callback";
/** A pending sign-in is accepted this long after it started on this device. */
export const PENDING_TTL_MS = 15 * 60_000;

export interface PendingSignIn {
  /** The address a magic link was requested for (absent for social sign-in). */
  email?: string;
  at: number;
  verifier: string;
}

/** Standard base64 to base64url without padding. */
export function toBase64Url(b64: string): string {
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Bytes to base64url (no padding). */
export function bytesToBase64Url(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return toBase64Url(btoa(s));
}

/** The https callback Better Auth redirects to after a magic link or an OAuth callback. */
export function handoffUrl(convexSiteUrl: string, challenge: string): string {
  return `${convexSiteUrl.replace(/\/+$/, "")}/mobile/auth/handoff?challenge=${encodeURIComponent(challenge)}`;
}

/** The redeem endpoint (`POST {code, verifier}` → `{cookie}`). */
export function redeemUrl(convexSiteUrl: string): string {
  return `${convexSiteUrl.replace(/\/+$/, "")}/mobile/auth/redeem`;
}

export function parsePending(raw: string | null, now: number): PendingSignIn | null {
  if (!raw) return null;
  try {
    const p = JSON.parse(raw) as Partial<PendingSignIn>;
    if (typeof p.at !== "number" || typeof p.verifier !== "string" || p.verifier.length < 43) return null;
    if (now - p.at > PENDING_TTL_MS || p.at - now > 60_000) return null;
    return { at: p.at, verifier: p.verifier, ...(typeof p.email === "string" ? { email: p.email } : {}) };
  } catch {
    return null;
  }
}

/** The single-use code from `akatsuki://auth/callback?code=…`, if well-formed. */
export function callbackCode(url: string): string | null {
  try {
    const code = new URL(url).searchParams.get("code");
    return code && /^[A-Za-z0-9_-]{43}$/.test(code) ? code : null;
  } catch {
    return null;
  }
}
