// Better Auth on mobile (ARCHITECTURE §6.4): the Expo plugin keeps the session cookie in SecureStore and handles
// social sign-in through expo-web-browser and the `akatsuki://` scheme; `convexClient()` mints the Convex JWT that
// Convex, the worker and apps/api accept; magic link and email OTP are the email methods.

import { setAccessTokenSource } from "@akatsuki/wire";
import { expoClient, getSetCookie, storageAdapter } from "@better-auth/expo/client";
import { convexClient } from "@convex-dev/better-auth/client/plugins";
import { emailOTPClient, magicLinkClient } from "better-auth/client/plugins";
import { createAuthClient } from "better-auth/react";
import { CryptoDigestAlgorithm, CryptoEncoding, digestStringAsync, getRandomBytes } from "expo-crypto";
import * as WebBrowser from "expo-web-browser";
import { ENV } from "../env";
import {
  bytesToBase64Url,
  callbackCode,
  HANDOFF_CALLBACK,
  handoffUrl,
  type PendingSignIn,
  parsePending,
  redeemUrl,
  toBase64Url,
} from "../vm/handoff";
import { secureDelete, secureGet, secureSet, secureSync } from "./secure";

export const AUTH_SCHEME = "akatsuki";
export const STORAGE_PREFIX = "akatsuki";
/** Where the handoff sends the app with a single-use code (§6.4, D-408). */
export const MAGIC_LINK_CALLBACK = HANDOFF_CALLBACK;
const COOKIE_KEY = `${STORAGE_PREFIX}_cookie`;
const PENDING_KEY = "auth.pending";

function makeClient(baseURL: string) {
  return createAuthClient({
    baseURL,
    plugins: [
      expoClient({ scheme: AUTH_SCHEME, storagePrefix: STORAGE_PREFIX, storage: secureSync }),
      convexClient(),
      magicLinkClient(),
      emailOTPClient(),
    ],
  });
}

export type AuthClient = ReturnType<typeof makeClient>;

export const authClient: AuthClient | null = ENV.env ? makeClient(ENV.env.convexSiteUrl) : null;

// A function source, not the client itself: the Better Auth client is a Proxy that answers every `in` check, so
// `setAccessTokenSource(authClient)` would mistake it for a ready token source (D-269).
if (authClient) {
  const client = authClient;
  setAccessTokenSource(async () => {
    const r = await client.convex.token({ fetchOptions: { throw: false } });
    return r.data?.token ?? null;
  });
}

function requireClient(): AuthClient {
  if (!authClient) throw new Error("Akatsuki is not configured: set the EXPO_PUBLIC_* variables.");
  return authClient;
}

/** The error text Better Auth returned, or a generic sentence. */
function message(err: { message?: string; status?: number } | null | undefined, fallback: string): string {
  if (!err) return fallback;
  if (err.status === 429) return "Too many attempts. Wait a minute and try again.";
  return err.message && err.message.length < 200 ? err.message : fallback;
}

/** Starts a handoff: a fresh verifier kept on this device, and the challenge that goes into the callback URL. */
async function startPending(email?: string): Promise<string> {
  const verifier = bytesToBase64Url(getRandomBytes(32));
  const pending: PendingSignIn = {
    at: Date.now(),
    verifier,
    ...(email ? { email: email.toLowerCase() } : {}),
  };
  await secureSet(PENDING_KEY, JSON.stringify(pending));
  const digest = await digestStringAsync(CryptoDigestAlgorithm.SHA256, verifier, {
    encoding: CryptoEncoding.BASE64,
  });
  return toBase64Url(digest);
}

async function readPending(): Promise<PendingSignIn | null> {
  return parsePending(await secureGet(PENDING_KEY).catch(() => null), Date.now());
}

function siteUrl(): string {
  if (!ENV.env) throw new Error("Akatsuki is not configured: set the EXPO_PUBLIC_* variables.");
  return ENV.env.convexSiteUrl;
}

export async function sendMagicLink(email: string): Promise<{ ok: true } | { ok: false; error: string }> {
  const challenge = await startPending(email);
  const callbackURL = handoffUrl(siteUrl(), challenge);
  const { error } = await requireClient().signIn.magicLink({ email, callbackURL });
  if (error) return { ok: false, error: message(error, "Could not send the link. Try again.") };
  return { ok: true };
}

export async function sendOtp(email: string): Promise<{ ok: true } | { ok: false; error: string }> {
  const { error } = await requireClient().emailOtp.sendVerificationOtp({ email, type: "sign-in" });
  if (error) return { ok: false, error: message(error, "Could not send the code. Try again.") };
  return { ok: true };
}

export async function verifyOtp(
  email: string,
  otp: string,
): Promise<{ ok: true } | { ok: false; error: string }> {
  const { error } = await requireClient().signIn.emailOtp({ email, otp });
  if (error) return { ok: false, error: message(error, "That code did not work. Check it and try again.") };
  return { ok: true };
}

export async function signInSocial(
  provider: "google" | "microsoft",
): Promise<{ ok: true } | { ok: false; error: string }> {
  // Not the Expo plugin's flow: that one ends with the session cookie in an `akatsuki://` URL, which any app on
  // Android can register for. The provider's callback ends on our https handoff instead (D-408).
  const challenge = await startPending();
  const callbackURL = handoffUrl(siteUrl(), challenge);
  const { data, error } = await requireClient().signIn.social({
    provider,
    callbackURL,
    errorCallbackURL: callbackURL,
    disableRedirect: true,
  });
  if (error || !data?.url) return { ok: false, error: message(error, "Sign-in did not start.") };
  const result = await WebBrowser.openAuthSessionAsync(data.url, MAGIC_LINK_CALLBACK);
  if (result.type !== "success") return { ok: false, error: "Sign-in was cancelled." };
  const code = callbackCode(result.url);
  if (!code) return { ok: false, error: "Sign-in did not finish." };
  return await acceptHandoff({ code });
}

/**
 * Completes a sign-in that came back as `akatsuki://auth/callback?code=…` (magic link or social, D-408): the code is
 * redeemed with the verifier this device kept, so a code intercepted by another app is useless. A magic link must
 * also arrive within 15 minutes of the request on this device, and the session must belong to the address that
 * was requested (a link someone else sent would otherwise sign this device into their account).
 */
export async function acceptHandoff(params: {
  code?: string;
  error?: string;
}): Promise<{ ok: true } | { ok: false; error: string }> {
  const client = requireClient();
  if (params.error)
    return { ok: false, error: "That sign-in link has expired or was already used. Request a new one." };
  if (!params.code) return { ok: false, error: "That sign-in link is incomplete. Request a new one." };
  const pending = await readPending();
  if (!pending) {
    // The social flow already redeemed this code (the auth session and the deep link both deliver it).
    const { data } = await client.getSession();
    if (data?.user) return { ok: true };
    return {
      ok: false,
      error: "Open the sign-in link on the device where you asked for it, within 15 minutes.",
    };
  }
  const res = await fetch(redeemUrl(siteUrl()), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ code: params.code, verifier: pending.verifier }),
  }).catch(() => null);
  await secureDelete(PENDING_KEY);
  const body = res?.ok ? ((await res.json().catch(() => null)) as { cookie?: unknown } | null) : null;
  if (typeof body?.cookie !== "string")
    return { ok: false, error: "That sign-in link has expired or was already used. Request a new one." };
  const store = storageAdapter(secureSync);
  await store.setItem(COOKIE_KEY, getSetCookie(body.cookie, store.getItem(COOKIE_KEY) ?? undefined));
  const { data } = await client.getSession();
  if (!data?.user || (pending.email && data.user.email.toLowerCase() !== pending.email)) {
    await client.signOut();
    return { ok: false, error: "That link was for a different email address." };
  }
  client.$store.notify("$sessionSignal");
  return { ok: true };
}

export async function signOutSession(): Promise<void> {
  try {
    await authClient?.signOut();
  } finally {
    await secureDelete(PENDING_KEY);
  }
}
