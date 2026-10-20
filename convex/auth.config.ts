// Convex auth providers (ARCHITECTURE §6.1, §6.6).
// 1. Better Auth user JWTs: RS256, iss CONVEX_SITE_URL, aud "convex", JWKS at /api/auth/convex/jwks.
// 2. Service JWTs from the sync worker and apps/api: ES256, separate issuers, data-URI JWKS from env.
//    They are added only when their env is set, so a local or fresh deployment still pushes.
import { getAuthConfigProvider } from "@convex-dev/better-auth/auth-config";
import type { AuthConfig, AuthProvider } from "convex/server";

const dataUri = (jwks: string) => `data:text/plain;charset=utf-8;base64,${btoa(jwks)}`;

function serviceProvider(issuer: string | undefined, jwks: string | undefined): AuthProvider[] {
  if (!issuer || !jwks) return [];
  return [
    {
      type: "customJwt",
      issuer,
      applicationID: "akatsuki-service",
      algorithm: "ES256",
      jwks: dataUri(jwks),
    },
  ];
}

export default {
  providers: [
    getAuthConfigProvider(),
    ...serviceProvider(process.env.WORKER_ISSUER, process.env.WORKER_SERVICE_JWKS), // "https://sync.akatsuki.vc"
    ...serviceProvider(process.env.API_ISSUER, process.env.API_SERVICE_JWKS), // "https://api.akatsuki.vc"
  ],
} satisfies AuthConfig;
