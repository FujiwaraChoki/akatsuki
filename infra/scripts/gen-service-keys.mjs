#!/usr/bin/env node
// Creates the ES256 service identities of ARCHITECTURE §6.6 (worker and apps/api) and, optionally, the
// worker's symmetric keys (§14.1). Private material is written only to files with mode 0600 in a gitignored
// directory and is never printed: stdout shows key ids, file paths and public JWK thumbprints.
//
//   node infra/scripts/gen-service-keys.mjs [--out infra/secrets/<stamp>] [--with-worker-secrets]
//        [--previous <dir of an earlier run>] [--worker-issuer URL] [--api-issuer URL]
//
// Output directory (default infra/secrets/<UTC timestamp>/, ignored by infra/.gitignore):
//   service-keys.json  everything, for the owner's password manager (0600)
//   sync.env.part      WORKER_ISSUER, WORKER_SERVICE_PRIVATE_KEY, WORKER_SERVICE_KID [+ worker secrets] (0600)
//   api.env.part       API_ISSUER, API_SERVICE_PRIVATE_KEY, API_SERVICE_KID, WORKER_ISSUER, WORKER_SERVICE_JWKS (0600)
//   convex.env.part    public halves only: WORKER_ISSUER, API_ISSUER, WORKER_SERVICE_JWKS, API_SERVICE_JWKS (0600)
//
// Append the .part files to /opt/akatsuki/sync.env and /opt/akatsuki/api.env on the server, and load
// convex.env.part with `npx convex env set --prod --from-file convex.env.part` (docs/DEPLOY.md).
//
// Rotation (§6.6): run again with --previous <old dir>. The new JWKS then publishes the new and the old
// public keys. Deploy Convex with it, switch the services to the new private key and kid, then run once
// more without --previous (or edit the JWKS) to drop the old key.
import { createHash, generateKeyPairSync, randomBytes } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

export const WORKER_SECRET_NAMES = [
  "STATE_KEY",
  "SPOOL_KEY",
  "LINK_SIGNING_KEY",
  "IMG_PROXY_KEY",
  "READ_RECEIPT_KEY",
];

/** One ES256 (P-256) service identity: PKCS#8 PEM private key plus its public JWK. */
export function makeServiceKey(prefix, now = new Date()) {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const stamp = now.toISOString().slice(0, 10).replaceAll("-", "");
  const kid = `${prefix}-${stamp}-${randomBytes(3).toString("hex")}`;
  const jwk = publicKey.export({ format: "jwk" });
  return {
    kid,
    privatePem: privateKey.export({ format: "pem", type: "pkcs8" }).toString(),
    publicJwk: { kty: jwk.kty, crv: jwk.crv, x: jwk.x, y: jwk.y, kid, alg: "ES256", use: "sig" },
  };
}

/** RFC 7638 thumbprint, safe to print. */
export function thumbprint(jwk) {
  const canonical = JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x, y: jwk.y });
  return createHash("sha256").update(canonical).digest("base64url");
}

export function jwks(...keys) {
  const seen = new Set();
  const out = [];
  for (const k of keys.flat()) {
    if (!k || seen.has(k.kid)) continue;
    seen.add(k.kid);
    out.push(k);
  }
  return JSON.stringify({ keys: out });
}

const oneLinePem = (pem) => pem.trim().replaceAll("\n", "\\n");
// Single quotes: Compose env_file and dotenv keep the content literally (JSON and the \n escapes intact).
const line = (name, value) => {
  if (String(value).includes("'")) throw new Error(`value for ${name} contains a single quote`);
  return `${name}='${value}'`;
};
const key32 = () => randomBytes(32).toString("base64");

/** Builds every output file's content. Pure apart from key generation. */
export function generate({
  workerIssuer = "https://sync.akatsuki.vc",
  apiIssuer = "https://api.akatsuki.vc",
  withWorkerSecrets = false,
  previous = null,
  now = new Date(),
} = {}) {
  const worker = makeServiceKey("worker", now);
  const api = makeServiceKey("api", now);
  const prevWorker = previous?.worker?.publicJwk ?? null;
  const prevApi = previous?.api?.publicJwk ?? null;
  const workerJwks = jwks(worker.publicJwk, prevWorker);
  const apiJwks = jwks(api.publicJwk, prevApi);

  const workerSecrets = {};
  if (withWorkerSecrets) {
    workerSecrets.TOKEN_ENC_KEYS = `k1:${key32()}`;
    for (const name of WORKER_SECRET_NAMES) workerSecrets[name] = key32();
  }

  const header = (what) =>
    [
      `# ${what}, generated ${now.toISOString()} by infra/scripts/gen-service-keys.mjs.`,
      "# Secret. Never commit, paste or print this file.",
    ].join("\n");

  const files = {
    "service-keys.json": `${JSON.stringify(
      {
        generatedAt: now.toISOString(),
        worker: { issuer: workerIssuer, ...worker, jwks: JSON.parse(workerJwks) },
        api: { issuer: apiIssuer, ...api, jwks: JSON.parse(apiJwks) },
        ...(withWorkerSecrets ? { workerSecrets } : {}),
      },
      null,
      2,
    )}\n`,
    "sync.env.part": [
      header("akatsuki-sync service identity"),
      line("WORKER_ISSUER", workerIssuer),
      line("WORKER_SERVICE_KID", worker.kid),
      line("WORKER_SERVICE_PRIVATE_KEY", oneLinePem(worker.privatePem)),
      ...Object.entries(workerSecrets).map(([n, v]) => line(n, v)),
      "",
    ].join("\n"),
    "api.env.part": [
      header("akatsuki-api service identity"),
      line("API_ISSUER", apiIssuer),
      line("API_SERVICE_KID", api.kid),
      line("API_SERVICE_PRIVATE_KEY", oneLinePem(api.privatePem)),
      line("WORKER_ISSUER", workerIssuer),
      line("WORKER_SERVICE_JWKS", workerJwks),
      "",
    ].join("\n"),
    "convex.env.part": [
      header("Convex deployment env (public halves only)"),
      line("WORKER_ISSUER", workerIssuer),
      line("API_ISSUER", apiIssuer),
      line("WORKER_SERVICE_JWKS", workerJwks),
      line("API_SERVICE_JWKS", apiJwks),
      "",
    ].join("\n"),
  };
  return { worker, api, workerJwks, apiJwks, files };
}

function parseArgs(argv) {
  const opts = {
    out: null,
    withWorkerSecrets: false,
    previous: null,
    workerIssuer: undefined,
    apiIssuer: undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === "--out") opts.out = next();
    else if (a === "--with-worker-secrets") opts.withWorkerSecrets = true;
    else if (a === "--previous") opts.previous = next();
    else if (a === "--worker-issuer") opts.workerIssuer = next();
    else if (a === "--api-issuer") opts.apiIssuer = next();
    else if (a === "--help" || a === "-h") opts.help = true;
    else throw new Error(`unknown argument: ${a}`);
  }
  return opts;
}

export function main(argv = process.argv.slice(2), log = console.info) {
  const opts = parseArgs(argv);
  if (opts.help) {
    log(
      "usage: gen-service-keys.mjs [--out DIR] [--with-worker-secrets] [--previous DIR] [--worker-issuer URL] [--api-issuer URL]",
    );
    return 0;
  }
  const stamp = new Date().toISOString().replaceAll(/[:.]/g, "-");
  const out = resolve(opts.out ?? join(repoRoot, "infra", "secrets", stamp));
  if (existsSync(join(out, "service-keys.json"))) {
    throw new Error(`refusing to overwrite existing keys in ${out}`);
  }
  let previous = null;
  if (opts.previous) {
    previous = JSON.parse(readFileSync(join(resolve(opts.previous), "service-keys.json"), "utf8"));
  }

  const result = generate({
    workerIssuer: opts.workerIssuer,
    apiIssuer: opts.apiIssuer,
    withWorkerSecrets: opts.withWorkerSecrets,
    previous,
  });

  mkdirSync(out, { recursive: true, mode: 0o700 });
  chmodSync(out, 0o700);
  for (const [name, content] of Object.entries(result.files)) {
    const path = join(out, name);
    writeFileSync(path, content, { mode: 0o600, flag: "wx" });
    chmodSync(path, 0o600);
  }

  const shown = relative(process.cwd(), out) || ".";
  log(`Wrote service keys to ${shown}/ (mode 0600; private keys are not printed).`);
  log(`  worker kid ${result.worker.kid}  thumbprint ${thumbprint(result.worker.publicJwk)}`);
  log(`  api    kid ${result.api.kid}  thumbprint ${thumbprint(result.api.publicJwk)}`);
  if (previous) log("  JWKS also publish the previous public keys (rotation step 1).");
  if (opts.withWorkerSecrets) log(`  worker secrets: TOKEN_ENC_KEYS, ${WORKER_SECRET_NAMES.join(", ")}`);
  log("Next: docs/DEPLOY.md, 'Service keys'.");
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    process.exitCode = main();
  } catch (err) {
    console.error(`gen-service-keys: ${err instanceof Error ? err.message : String(err)}`);
    process.exitCode = 1;
  }
}
