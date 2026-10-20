// User and service token verification (ARCHITECTURE §6.5, §6.6, D-048), with jose.
import {
  createLocalJWKSet,
  createRemoteJWKSet,
  importPKCS8,
  type JWTVerifyGetKey,
  jwtVerify,
  SignJWT,
} from "jose";

export interface VerifiedUser {
  userId: string;
  /** From the `email` claim (definePayload); used only for Stripe. */
  email?: string;
}

export class AuthError extends Error {
  override name = "AuthError";
}

const CLOCK_TOLERANCE_S = 30;

/**
 * Verifies Better Auth user JWTs: RS256 only, iss = CONVEX_SITE_URL, aud = "convex", exp/nbf with 30 s
 * leeway, non-empty sub. JWKS from `${CONVEX_SITE_URL}/api/auth/convex/jwks`, cached for 1 hour and
 * refetched at most once a minute when an unknown kid appears.
 */
export function createUserVerifier(opts: { convexSiteUrl: string; jwks?: JWTVerifyGetKey }) {
  const issuer = opts.convexSiteUrl.replace(/\/+$/, "");
  const jwks =
    opts.jwks ??
    createRemoteJWKSet(new URL(`${issuer}/api/auth/convex/jwks`), {
      cacheMaxAge: 60 * 60 * 1000,
      cooldownDuration: 60 * 1000,
      timeoutDuration: 5000,
    });
  return async function verifyUser(token: string): Promise<VerifiedUser> {
    try {
      const { payload } = await jwtVerify(token, jwks, {
        algorithms: ["RS256"],
        issuer,
        audience: "convex",
        clockTolerance: CLOCK_TOLERANCE_S,
        requiredClaims: ["exp", "sub"],
      });
      if (typeof payload.sub !== "string" || payload.sub.length === 0) throw new AuthError("empty sub");
      const email = typeof payload.email === "string" ? payload.email : undefined;
      return email ? { userId: payload.sub, email } : { userId: payload.sub };
    } catch (e) {
      throw e instanceof AuthError ? e : new AuthError("invalid token");
    }
  };
}

/** Verifies worker service tokens for /internal/*: ES256, iss WORKER_ISSUER, aud "akatsuki-api". */
export function createWorkerVerifier(opts: { workerServiceJwks: string; workerIssuer: string }) {
  const jwks = createLocalJWKSet(JSON.parse(opts.workerServiceJwks));
  return async function verifyWorker(token: string): Promise<{ sub: string }> {
    try {
      const { payload } = await jwtVerify(token, jwks, {
        algorithms: ["ES256"],
        issuer: opts.workerIssuer,
        audience: "akatsuki-api",
        clockTolerance: CLOCK_TOLERANCE_S,
        requiredClaims: ["exp", "sub"],
      });
      if (typeof payload.sub !== "string" || !payload.sub.startsWith("worker:"))
        throw new AuthError("bad sub");
      return { sub: payload.sub };
    } catch (e) {
      throw e instanceof AuthError ? e : new AuthError("invalid token");
    }
  };
}

/** `Authorization: Bearer <token>` → token, or null. Cookies are never accepted (§14.5). */
export function bearerToken(header: string | undefined): string | null {
  if (!header) return null;
  const m = /^Bearer[ ]+([A-Za-z0-9\-_.]+)$/.exec(header.trim());
  return m ? m[1]! : null;
}

/**
 * apps/api's own identity towards Convex (§6.6): ES256, iss API_ISSUER, aud "akatsuki-service", sub "api",
 * 10-minute tokens, re-minted every 5 minutes.
 */
export function createServiceTokenSource(opts: {
  privateKeyPem: string;
  kid: string;
  issuer: string;
  now?: () => number;
}) {
  const now = opts.now ?? Date.now;
  let keyPromise: ReturnType<typeof importPKCS8> | undefined;
  let cached: { token: string; mintedAt: number } | undefined;
  return async function serviceToken(): Promise<string> {
    const t = now();
    if (cached && t - cached.mintedAt < 5 * 60 * 1000) return cached.token;
    keyPromise ??= importPKCS8(opts.privateKeyPem, "ES256");
    const iat = Math.floor(t / 1000);
    const token = await new SignJWT({})
      .setProtectedHeader({ alg: "ES256", kid: opts.kid, typ: "JWT" })
      .setIssuer(opts.issuer)
      .setAudience("akatsuki-service")
      .setSubject("api")
      .setIssuedAt(iat)
      .setExpirationTime(iat + 600)
      .sign(await keyPromise);
    cached = { token, mintedAt: t };
    return token;
  };
}
