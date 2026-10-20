// Better Auth on Convex (ARCHITECTURE §6.1), default component with no Local Install (D-034).
//
// Pinned pair: @convex-dev/better-auth 0.12.5 + better-auth 1.6.33 (peer range >=1.6.11 <1.7.0, D-035).
// Login methods: magic link and email OTP through SES (internal.email.send), plus env-gated social
// login (Google, Microsoft, GitHub; openid/email/profile only, separate from mailbox OAuth).
// Session JWTs for Convex, the sync worker and apps/api: RS256 via the `convex` plugin, served at
// /api/auth/convex/token and /api/auth/convex/jwks.
// Passkeys are not enabled: the passkey plugin needs a `passkey` table that the default component
// schema does not have, which would force a Local Install (rejected in D-034).
import { expo } from "@better-auth/expo";
import { type AuthFunctions, createClient, type GenericCtx } from "@convex-dev/better-auth";
import { convex, crossDomain } from "@convex-dev/better-auth/plugins";
import { type BetterAuthOptions, betterAuth } from "better-auth";
import { APIError, createAuthMiddleware, getIp } from "better-auth/api";
import { emailOTP, magicLink } from "better-auth/plugins";
import { components, internal } from "./_generated/api";
import type { DataModel } from "./_generated/dataModel";
import authConfig from "./auth.config";
import type { EmailTemplate } from "./lib/email";
import { createUserRows } from "./lib/userRows";

const authFunctions: AuthFunctions = internal.auth;

export const authComponent = createClient<DataModel>(components.betterAuth, {
  authFunctions,
  triggers: {
    user: {
      onCreate: async (ctx, user) => {
        // user._id === JWT sub === userId everywhere in the app tables.
        await createUserRows(ctx, { userId: user._id, email: user.email, name: user.name });
      },
      // Better Auth's own deleteUser (enabled below): the app rows go with the login (D-393).
      onDelete: async (ctx, user) => {
        await ctx.scheduler.runAfter(0, internal.accounts.purgeUser, { userId: user._id });
      },
    },
  },
});

export const { onCreate, onUpdate, onDelete } = authComponent.triggersApi();

/** Queues a transactional email; every send goes through internal.email.send (D-047). */
async function sendEmail(
  ctx: GenericCtx<DataModel>,
  template: EmailTemplate,
  to: string,
  vars: Record<string, string>,
) {
  if (!("scheduler" in ctx)) throw new Error("email can only be sent from a mutation or action");
  await ctx.scheduler.runAfter(0, internal.email.send, { template, to, vars });
}

function socialProviders() {
  const env = process.env;
  return {
    ...(env.GOOGLE_LOGIN_CLIENT_ID && {
      google: { clientId: env.GOOGLE_LOGIN_CLIENT_ID, clientSecret: env.GOOGLE_LOGIN_CLIENT_SECRET ?? "" },
    }),
    ...(env.MICROSOFT_LOGIN_CLIENT_ID && {
      microsoft: {
        clientId: env.MICROSOFT_LOGIN_CLIENT_ID,
        clientSecret: env.MICROSOFT_LOGIN_CLIENT_SECRET ?? "",
        tenantId: "common",
      },
    }),
    ...(env.GITHUB_LOGIN_CLIENT_ID && {
      github: { clientId: env.GITHUB_LOGIN_CLIENT_ID, clientSecret: env.GITHUB_LOGIN_CLIENT_SECRET ?? "" },
    }),
  };
}

export const createAuth = (ctx: GenericCtx<DataModel>) => {
  const siteUrl = process.env.SITE_URL ?? "https://app.akatsuki.vc";
  return betterAuth({
    baseURL: process.env.CONVEX_SITE_URL,
    secret: process.env.BETTER_AUTH_SECRET,
    database: authComponent.adapter(ctx),
    trustedOrigins: [
      siteUrl,
      "akatsuki://",
      "http://127.0.0.1:*",
      ...(process.env.DEV_ORIGINS?.split(",")
        .map((s) => s.trim())
        .filter(Boolean) ?? []),
    ],
    socialProviders: socialProviders(),
    // Settings → Delete account ends with deleteUser; `me.deleteAccount` already removed the user server-side,
    // so this only matters for clients that call it alone (fresh session required, D-292, D-393).
    user: { deleteUser: { enabled: true } },
    // Per-IP limits in the app's `authRateLimits` table (convex/authLimits.ts): in-memory state does not
    // survive across Convex isolates, the default only runs when NODE_ENV is production (D-394), and the
    // component's own table cannot be swept without a full scan (D-401). The sign-in emails also have
    // a per-recipient limit in internal.email.send, which holds whatever the IP looks like. Every rule is
    // per IP only: a request without a trustworthy client IP is not IP-limited at all, because Better Auth
    // would otherwise count every such client in one shared bucket per path (D-401).
    rateLimit: {
      enabled: true,
      window: 60,
      max: 100,
      customRules: rateLimitRules(),
      customStorage: rateLimitStorage(ctx),
    },
    advanced: { ipAddress: ipAddressOptions() },
    // Sign-in redirects end on http(s) only. The Expo plugin would put the session cookie into a redirect to a
    // trusted custom scheme (`akatsuki://`), which any Android app can register for; the mobile app uses the
    // https handoff instead (convex/mobileAuth.ts, D-408). `akatsuki://` stays trusted as the app's request origin.
    hooks: { before: createAuthMiddleware(async (ctx) => rejectSchemeCallbacks(ctx.body, ctx.query)) },
    logger: { log: authLog },
    plugins: [
      crossDomain({ siteUrl }),
      expo(),
      magicLink({ sendMagicLink: ({ email, url }) => sendEmail(ctx, "magic_link", email, { url }) }),
      emailOTP({ sendVerificationOTP: ({ email, otp }) => sendEmail(ctx, "otp", email, { otp }) }),
      convex({
        authConfig,
        jwt: {
          expirationSeconds: 900,
          definePayload: ({ user }) => ({ email: user.email, name: user.name }),
        },
      }),
    ],
  });
};

type IpOptions = NonNullable<NonNullable<BetterAuthOptions["advanced"]>["ipAddress"]>;
type RateRule = { window: number; max: number };

/**
 * Where the client IP comes from. `AUTH_TRUSTED_PROXIES` (comma-separated IPs or CIDRs, optional) lets a
 * multi-hop `x-forwarded-for` resolve: without it Better Auth trusts only a single-value header (D-401).
 */
export function ipAddressOptions(env: Record<string, string | undefined> = process.env): IpOptions {
  const trustedProxies = (env.AUTH_TRUSTED_PROXIES ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return {
    ipAddressHeaders: ["x-forwarded-for", "x-real-ip"],
    ipv6Subnet: 64,
    ...(trustedProxies.length > 0 && { trustedProxies }),
  };
}

/**
 * Better Auth rate-limit rules (D-394, D-401). Each rule applies only when the request has a resolvable client
 * IP. Without one (no forwarding proxy, as on the local harness, or a header Better Auth cannot trust) Better
 * Auth 1.6.33 would key every client to one shared `no-trusted-ip` bucket per path, so three sign-in emails a
 * minute from anyone would block everyone's sign-in; such requests are left to the per-recipient email limit.
 * The catch-all keeps Better Auth's own defaults (`current`) under the same condition.
 */
export function rateLimitRules(
  env: Record<string, string | undefined> = process.env,
): Record<string, (req: Request, current: RateRule) => RateRule | false> {
  const ipAddress = ipAddressOptions(env);
  const options = { advanced: { ipAddress } } as BetterAuthOptions;
  const headers = ipAddress.ipAddressHeaders ?? [];
  // Better Auth answers 127.0.0.1 when NODE_ENV is development or test and no header resolves; that is not
  // a client address either.
  const hasClientIp = (req: Request) => {
    const ip = getIp(req, options);
    if (!ip) return false;
    return ip !== "127.0.0.1" || headers.some((h) => req.headers.get(h)?.includes(ip));
  };
  const perIp = (rule?: RateRule) => (req: Request, current: RateRule) =>
    hasClientIp(req) ? (rule ?? current) : false;
  return {
    "/sign-in/magic-link": perIp({ window: 60, max: 3 }),
    "/email-otp/send-verification-otp": perIp({ window: 60, max: 3 }),
    "/sign-in/email-otp": perIp({ window: 60, max: 10 }),
    "/**": perIp(),
  };
}

type RateLimitStorage = NonNullable<NonNullable<BetterAuthOptions["rateLimit"]>["customStorage"]>;

/**
 * Counters in `authRateLimits` through one atomic mutation per request. Only HTTP actions (which have
 * `runMutation`) reach the rate limiter; any other context lets the request through.
 */
export function rateLimitStorage(ctx: GenericCtx<DataModel>): RateLimitStorage {
  const run = "runMutation" in ctx ? ctx.runMutation : null;
  return {
    // `consume` replaces get/set (Better Auth uses get/set only for storages without it).
    get: async () => null,
    set: async () => undefined,
    consume: async (key, rule) => {
      if (!run) return { allowed: true, retryAfter: null };
      return await run(internal.authLimits.consume, { key, window: rule.window, max: rule.max });
    },
  };
}

/**
 * Better Auth's log lines, minus one: the "could not determine a client IP … single shared per-path bucket"
 * warning, which it prints before evaluating `customRules`. Our rules skip IP limiting for such requests
 * (D-401), so the warning describes something that does not happen.
 */
export function authLog(level: "debug" | "info" | "warn" | "error", message: string, ...args: unknown[]) {
  if (level === "warn" && message.includes("could not determine a client IP")) return;
  const out = level === "error" ? console.error : level === "warn" ? console.warn : console.info;
  out(`[Better Auth] ${message}`, ...args);
}

const CALLBACK_FIELDS = ["callbackURL", "newUserCallbackURL", "errorCallbackURL"] as const;

/** Refuses sign-in callbacks that are not http(s) (absolute) or a path (D-408). */
export function rejectSchemeCallbacks(...sources: unknown[]): void {
  for (const src of sources) {
    if (!src || typeof src !== "object") continue;
    for (const f of CALLBACK_FIELDS) {
      const value = (src as Record<string, unknown>)[f];
      if (typeof value !== "string" || value.startsWith("/")) continue;
      if (!/^https?:\/\//i.test(value))
        throw new APIError("BAD_REQUEST", { message: "Sign-in callbacks must be https or a path." });
    }
  }
}
