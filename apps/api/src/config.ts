// Runtime configuration from the environment (names in ARCHITECTURE §18 and infra/api.env.example).
// Values are never logged; `describeConfig` reports only which optional integrations are present.
import { z } from "zod";

const csv = (v: string | undefined) =>
  (v ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

/** The Akatsuki Pro monthly price's lookup key (§13.1); the price id is resolved at runtime. */
export const DEFAULT_PRICE_LOOKUP_KEY = "akatsuki_pro_monthly";

const schema = z.object({
  CONVEX_URL: z.url(),
  CONVEX_SITE_URL: z.url(),
  API_ISSUER: z.string().min(1).default("https://api.akatsuki.vc"),
  API_SERVICE_PRIVATE_KEY: z.string().min(1),
  API_SERVICE_KID: z.string().min(1),
  WORKER_SERVICE_JWKS: z.string().optional(),
  WORKER_ISSUER: z.string().min(1).default("https://sync.akatsuki.vc"),
  OPENROUTER_API_KEY: z.string().optional(),
  OPENAI_API_KEY: z.string().optional(),
  STRIPE_SECRET_KEY: z.string().optional(),
  STRIPE_WEBHOOK_SECRET: z.string().optional(),
  STRIPE_PRICE_LOOKUP_KEY: z.string().min(1).default(DEFAULT_PRICE_LOOKUP_KEY),
  SITE_URL: z.url().default("https://app.akatsuki.vc"),
  CORS_ORIGINS: z.string().optional(),
  AKATSUKI_BILLING: z.enum(["on", "off"]).default("on"),
  BIND_ADDR: z.string().default("0.0.0.0:8787"),
});

/**
 * Every environment variable apps/api reads, and nothing else (zod drops unknown keys). Pinned by
 * test/config.test.ts to the apps/api row of ARCHITECTURE §18, infra/api.env.example and .env.example.
 */
export const API_ENV_NAMES = Object.keys(schema.shape) as (keyof typeof schema.shape)[];

export interface ApiConfig {
  convexUrl: string;
  convexSiteUrl: string;
  apiIssuer: string;
  apiServicePrivateKey: string;
  apiServiceKid: string;
  workerServiceJwks: string | undefined;
  workerIssuer: string;
  openrouterApiKey: string | undefined;
  openaiApiKey: string | undefined;
  stripeSecretKey: string | undefined;
  stripeWebhookSecret: string | undefined;
  stripePriceLookupKey: string;
  siteUrl: string;
  corsOrigins: string[];
  billing: boolean;
  host: string;
  port: number;
}

/** Exact-origin CORS allow-list: CORS_ORIGINS, else the web app plus the local Vite dev server. */
export const DEFAULT_CORS_ORIGINS = [
  "https://app.akatsuki.vc",
  "http://localhost:5173",
  "http://127.0.0.1:5173",
];

export function loadConfig(env: Record<string, string | undefined> = process.env): ApiConfig {
  const parsed = schema.safeParse(env);
  if (!parsed.success) {
    // Only variable names, never values.
    const names = [...new Set(parsed.error.issues.map((i) => String(i.path[0])))].join(", ");
    throw new Error(`invalid or missing environment variables: ${names}`);
  }
  const e = parsed.data;
  const [host, portText] = splitHostPort(e.BIND_ADDR);
  const origins = csv(e.CORS_ORIGINS);
  return {
    convexUrl: e.CONVEX_URL,
    convexSiteUrl: e.CONVEX_SITE_URL.replace(/\/+$/, ""),
    apiIssuer: e.API_ISSUER,
    apiServicePrivateKey: e.API_SERVICE_PRIVATE_KEY.replaceAll("\\n", "\n"),
    apiServiceKid: e.API_SERVICE_KID,
    workerServiceJwks: e.WORKER_SERVICE_JWKS || undefined,
    workerIssuer: e.WORKER_ISSUER,
    openrouterApiKey: e.OPENROUTER_API_KEY || undefined,
    openaiApiKey: e.OPENAI_API_KEY || undefined,
    stripeSecretKey: e.STRIPE_SECRET_KEY || undefined,
    stripeWebhookSecret: e.STRIPE_WEBHOOK_SECRET || undefined,
    stripePriceLookupKey: e.STRIPE_PRICE_LOOKUP_KEY,
    siteUrl: e.SITE_URL.replace(/\/+$/, ""),
    corsOrigins: origins.length > 0 ? origins : DEFAULT_CORS_ORIGINS,
    billing: e.AKATSUKI_BILLING === "on",
    host,
    port: Number(portText),
  };
}

function splitHostPort(addr: string): [string, string] {
  const i = addr.lastIndexOf(":");
  if (i <= 0) throw new Error("BIND_ADDR must be host:port");
  const port = addr.slice(i + 1);
  if (!/^\d{1,5}$/.test(port)) throw new Error("BIND_ADDR must be host:port");
  return [addr.slice(0, i).replace(/^\[|\]$/g, ""), port];
}

export function describeConfig(c: ApiConfig) {
  return {
    managedAi: Boolean(c.openrouterApiKey),
    transcription: Boolean(c.openaiApiKey),
    billing: c.billing,
    stripe: Boolean(c.stripeSecretKey && c.stripeWebhookSecret),
    workerTriage: Boolean(c.workerServiceJwks),
    corsOrigins: c.corsOrigins.length,
  };
}
