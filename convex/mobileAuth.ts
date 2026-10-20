// Mobile sign-in handoff (ARCHITECTURE §6.4, D-408): the session never travels in a custom-scheme URL.
//
// Android does not reserve `akatsuki://`: any installed app can register it and receive the redirect. So magic links
// and social sign-in on mobile end on this deployment's own https origin instead:
//   1. The app makes a random `verifier`, keeps it in SecureStore, and starts sign-in with
//      `callbackURL = <site>/mobile/auth/handoff?challenge=<base64url(sha256(verifier))>`.
//   2. Better Auth verifies the link (or the OAuth callback) and redirects there with a 3-minute one-time token
//      (`ott`, crossDomain plugin). `handoff` stores {challenge, ott} under a random single-use `code` and redirects
//      to `akatsuki://auth/callback?code=<code>`: only the code crosses the custom scheme.
//   3. The app posts {code, verifier} to `redeem`. The verifier must hash to the stored challenge (one attempt: a
//      wrong verifier burns the code); the ott is then exchanged for the session cookie server-side and returned in
//      the response body to the app that started the flow.
// An app that intercepts the redirect holds a code it cannot redeem.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import { httpAction, internalMutation } from "./_generated/server";
import { createAuth } from "./auth";

/** Where the handoff sends the app. */
export const MOBILE_CALLBACK = "akatsuki://auth/callback";
/** A code is redeemable this long (the ott inside lives 3 minutes). */
export const HANDOFF_TTL_MS = 3 * 60 * 1000;
const B64URL = /^[A-Za-z0-9_-]+$/;

function b64url(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** base64url(sha256(text)), the PKCE S256 transform (RFC 7636 §4.2). */
export async function s256(text: string): Promise<string> {
  return b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))));
}

function redirect(location: string): Response {
  return new Response(null, { status: 302, headers: { location, "cache-control": "no-store" } });
}

export const createHandoff = internalMutation({
  args: { codeHash: v.string(), challenge: v.string(), ott: v.string() },
  returns: v.null(),
  handler: async (ctx, args) => {
    const now = Date.now();
    await ctx.db.insert("authHandoffs", { ...args, expiresAt: now + HANDOFF_TTL_MS });
    // Expired rows of earlier flows go as new ones arrive (bounded work per insert).
    for (const old of await ctx.db
      .query("authHandoffs")
      .withIndex("by_expires", (q) => q.lt("expiresAt", now))
      .take(20))
      await ctx.db.delete(old._id);
    return null;
  },
});

/** Takes the handoff for `codeHash` (single use, whatever the verifier) and says whether it may be redeemed. */
export const takeHandoff = internalMutation({
  args: { codeHash: v.string(), challenge: v.string() },
  returns: v.union(v.object({ ott: v.string() }), v.null()),
  handler: async (ctx, { codeHash, challenge }) => {
    const row = await ctx.db
      .query("authHandoffs")
      .withIndex("by_code_hash", (q) => q.eq("codeHash", codeHash))
      .unique();
    if (!row) return null;
    await ctx.db.delete(row._id);
    if (row.expiresAt < Date.now() || row.challenge !== challenge) return null;
    return { ott: row.ott };
  },
});

/** `GET /mobile/auth/handoff?challenge=&ott=` (Better Auth's redirect after a magic link or OAuth callback). */
export const handoff = httpAction(async (ctx, request) => {
  const url = new URL(request.url);
  const challenge = url.searchParams.get("challenge") ?? "";
  const ott = url.searchParams.get("ott") ?? "";
  if (
    url.searchParams.get("error") ||
    !ott ||
    challenge.length !== 43 ||
    !B64URL.test(challenge) ||
    ott.length > 200
  )
    return redirect(`${MOBILE_CALLBACK}?error=expired`);
  const code = b64url(crypto.getRandomValues(new Uint8Array(32)));
  await ctx.runMutation(internal.mobileAuth.createHandoff, { codeHash: await s256(code), challenge, ott });
  return redirect(`${MOBILE_CALLBACK}?code=${code}`);
});

/** `POST /mobile/auth/redeem {code, verifier}` → `{cookie}` (the Better Auth session cookie), or 400. */
export const redeem = httpAction(async (ctx, request) => {
  const bad = () =>
    new Response(
      JSON.stringify({ error: { code: "invalid_argument", message: "Request a new sign-in link." } }),
      {
        status: 400,
        headers: { "content-type": "application/json", "cache-control": "no-store" },
      },
    );
  let body: { code?: unknown; verifier?: unknown };
  try {
    body = (await request.json()) as typeof body;
  } catch {
    return bad();
  }
  const { code, verifier } = body;
  if (typeof code !== "string" || typeof verifier !== "string") return bad();
  if (
    code.length !== 43 ||
    !B64URL.test(code) ||
    verifier.length < 43 ||
    verifier.length > 128 ||
    !B64URL.test(verifier)
  )
    return bad();
  const taken = await ctx.runMutation(internal.mobileAuth.takeHandoff, {
    codeHash: await s256(code),
    challenge: await s256(verifier),
  });
  if (!taken) return bad();
  // Exchange the one-time token for the session the same way the web client does, inside this action.
  const site = (process.env.CONVEX_SITE_URL ?? "").replace(/\/+$/, "");
  const auth = createAuth(ctx);
  const res = await auth.handler(
    new Request(`${site}/api/auth/cross-domain/one-time-token/verify`, {
      method: "POST",
      headers: { "content-type": "application/json", origin: site },
      body: JSON.stringify({ token: taken.ott }),
    }),
  );
  const cookie = res.headers.get("set-cookie");
  if (!res.ok || !cookie) return bad();
  return new Response(JSON.stringify({ cookie }), {
    status: 200,
    headers: { "content-type": "application/json", "cache-control": "no-store" },
  });
});
