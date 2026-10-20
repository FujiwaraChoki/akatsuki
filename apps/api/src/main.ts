// apps/api entry point (Docker: node dist/server.mjs). Reads env, wires dependencies, serves Hono on Node.
import { createOpenAI } from "@ai-sdk/openai";
import { createManagedModel } from "@akatsuki/ai";
import { serve } from "@hono/node-server";
import { createApp } from "./app.ts";
import { createServiceTokenSource, createUserVerifier, createWorkerVerifier } from "./auth.ts";
import { describeConfig, loadConfig } from "./config.ts";
import { createConvexGateway } from "./convex.ts";
import { createStripeBilling } from "./stripe.ts";

// AI SDK warnings go to the console with model ids only, but nothing reaches logs except our own lines.
(globalThis as { AI_SDK_LOG_WARNINGS?: boolean }).AI_SDK_LOG_WARNINGS = false;

const config = loadConfig();
const serviceToken = createServiceTokenSource({
  privateKeyPem: config.apiServicePrivateKey,
  kid: config.apiServiceKid,
  issuer: config.apiIssuer,
});
const openrouterKey = config.openrouterApiKey;
const openaiKey = config.openaiApiKey;
// Managed dictation calls OpenAI directly: the OpenRouter ZDR/data-collection controls do not apply, and
// zero data retention for this OpenAI organisation is not confirmed (ARCHITECTURE §12.7, D-128).
const openai = openaiKey ? createOpenAI({ apiKey: openaiKey }) : undefined;

const app = createApp({
  config,
  verifyUser: createUserVerifier({ convexSiteUrl: config.convexSiteUrl }),
  verifyWorker: config.workerServiceJwks
    ? createWorkerVerifier({ workerServiceJwks: config.workerServiceJwks, workerIssuer: config.workerIssuer })
    : undefined,
  convex: createConvexGateway({ convexUrl: config.convexUrl, serviceToken }),
  languageModel: openrouterKey ? (modelId) => createManagedModel(modelId, openrouterKey) : undefined,
  transcriptionModel: openai ? (modelId) => openai.transcription(modelId) : undefined,
  billing:
    config.billing && config.stripeSecretKey && config.stripeWebhookSecret
      ? createStripeBilling({
          secretKey: config.stripeSecretKey,
          webhookSecret: config.stripeWebhookSecret,
          priceLookupKey: config.stripePriceLookupKey,
          siteUrl: config.siteUrl,
        })
      : undefined,
});

const server = serve({ fetch: app.fetch, hostname: config.host, port: config.port }, (info) => {
  console.info(JSON.stringify({ msg: "akatsuki-api listening", port: info.port, ...describeConfig(config) }));
});

for (const signal of ["SIGTERM", "SIGINT"] as const) {
  process.on(signal, () => {
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(0), 10_000).unref();
  });
}
