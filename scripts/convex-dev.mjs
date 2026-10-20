#!/usr/bin/env node
// Local self-hosted Convex harness: start, stop, reset, admin key -> .env.local.
//
//   node scripts/convex-dev.mjs up      start the backend (Docker if usable, else the native binary)
//   node scripts/convex-dev.mjs down    stop it (data is kept)
//   node scripts/convex-dev.mjs reset   stop it and delete all local data
//   node scripts/convex-dev.mjs status  print the runtime in use and whether it answers
//
// Secrets: the admin key is appended to .env.local and never printed. .env.local is never read
// by this script (repo rule), so a marker under .convex-dev/ records which key was appended.
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash, generateKeyPairSync, randomBytes } from "node:crypto";
import {
  appendFileSync,
  chmodSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const stateDir = join(root, ".convex-dev");
const compose = ["compose", "-f", join(root, "infra/docker-compose.dev.yml")];

// Keep in sync with infra/docker-compose.dev.yml (same upstream revision as the pinned images).
const RELEASE = "precompiled-2026-10-10-72ce13c";
const PORT = Number(process.env.AKATSUKI_CONVEX_PORT ?? 3410);
const SITE_PORT = Number(process.env.AKATSUKI_CONVEX_SITE_PORT ?? 3411);
const INSTANCE_NAME = process.env.AKATSUKI_CONVEX_INSTANCE_NAME ?? "akatsuki-dev";
const URL_ = `http://127.0.0.1:${PORT}`;

const nativeDir = join(stateDir, "native");
const binPath = join(nativeDir, "bin", "convex-local-backend");
const pidFile = join(nativeDir, "backend.pid");
const secretFile = join(nativeDir, "instance_secret");
const marker = join(stateDir, "env-local-written");
const runtimeFile = join(stateDir, "runtime");

const log = (msg) => console.info(`[convex-dev] ${msg}`);

function dockerUsable() {
  const r = spawnSync("docker", ["info", "--format", "{{.ServerVersion}}"], { stdio: "pipe" });
  return r.status === 0;
}

async function waitForVersion(timeoutMs = 60_000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`${URL_}/version`);
      if (res.ok) return true;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

function ensureNativeBinary() {
  if (existsSync(binPath)) return;
  mkdirSync(join(nativeDir, "bin"), { recursive: true });
  const zip = join(nativeDir, "backend.zip");
  const asset = "convex-local-backend-x86_64-unknown-linux-gnu.zip";
  log(`downloading ${asset} (${RELEASE})`);
  execFileSync(
    "curl",
    [
      "-fsSL",
      "-o",
      zip,
      `https://github.com/get-convex/convex-backend/releases/download/${RELEASE}/${asset}`,
    ],
    {
      stdio: "inherit",
    },
  );
  execFileSync("unzip", ["-o", "-q", zip, "-d", join(nativeDir, "bin")], { stdio: "inherit" });
  chmodSync(binPath, 0o755);
  rmSync(zip);
}

function nativeSecret() {
  if (!existsSync(secretFile)) {
    mkdirSync(nativeDir, { recursive: true });
    writeFileSync(secretFile, randomBytes(32).toString("hex"), { mode: 0o600 });
  }
  return readFileSync(secretFile, "utf8").trim();
}

function nativeRunning() {
  if (!existsSync(pidFile)) return false;
  const pid = Number(readFileSync(pidFile, "utf8"));
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function upNative() {
  ensureNativeBinary();
  const secret = nativeSecret();
  if (!nativeRunning()) {
    const dataDir = join(nativeDir, "data");
    mkdirSync(join(dataDir, "storage"), { recursive: true });
    const out = openSync(join(nativeDir, "backend.log"), "a");
    const child = spawn(
      binPath,
      [
        "--interface",
        "127.0.0.1",
        "--port",
        String(PORT),
        "--site-proxy-port",
        String(SITE_PORT),
        "--instance-name",
        INSTANCE_NAME,
        "--instance-secret",
        secret,
        "--local-storage",
        join(dataDir, "storage"),
        "--convex-origin",
        URL_,
        "--convex-site",
        `http://127.0.0.1:${SITE_PORT}`,
        "--disable-beacon",
        join(dataDir, "convex_local_backend.sqlite3"),
      ],
      { detached: true, stdio: ["ignore", out, out] },
    );
    child.unref();
    writeFileSync(pidFile, String(child.pid));
    log(`started native backend (pid ${child.pid}); log: .convex-dev/native/backend.log`);
  }
  if (!(await waitForVersion()))
    throw new Error("backend did not answer on /version; see .convex-dev/native/backend.log");
  const key = execFileSync(
    binPath,
    ["keygen", "admin-key", "--instance-name", INSTANCE_NAME, "--instance-secret", secret],
    {
      encoding: "utf8",
    },
  ).trim();
  return key;
}

async function upDocker() {
  execFileSync("docker", [...compose, "up", "-d", "--wait"], { stdio: "inherit" });
  if (!(await waitForVersion())) throw new Error("backend did not answer on /version");
  const out = execFileSync(
    "docker",
    [...compose, "exec", "-T", "convex-backend", "./generate_admin_key.sh"],
    { encoding: "utf8" },
  );
  // The script prints a banner line, then the key on the last line.
  const key = out.trim().split("\n").pop()?.trim() ?? "";
  if (!key.includes("|")) throw new Error("could not parse the admin key from generate_admin_key.sh");
  return key;
}

function writeEnvLocal(adminKey, runtime) {
  // Admin keys are not deterministic, so the marker tracks the backend instance, not the key.
  const instance = runtime === "native" ? nativeSecret() : "docker";
  const fingerprint = createHash("sha256").update(`${runtime}\n${URL_}\n${instance}`).digest("hex");
  if (existsSync(marker) && readFileSync(marker, "utf8").trim() === fingerprint) {
    log(".env.local already has this deployment's URL and admin key");
    return;
  }
  // Later lines win in dotenv parsers, so appending replaces an older pair.
  appendFileSync(
    join(root, ".env.local"),
    `\n# Local self-hosted Convex (scripts/convex-dev.mjs, ${new Date().toISOString()})\nCONVEX_SELF_HOSTED_URL=${URL_}\nCONVEX_SELF_HOSTED_ADMIN_KEY=${adminKey}\n`,
    { mode: 0o600 },
  );
  writeFileSync(marker, fingerprint, { mode: 0o600 });
  log("appended CONVEX_SELF_HOSTED_URL and CONVEX_SELF_HOSTED_ADMIN_KEY to .env.local (value not shown)");
}

// ───────────── deployment env for local dev ─────────────
// Convex refuses to push while an env var used by auth.config.ts is unset, so the harness provisions
// local-only values: a Better Auth secret and ES256 service key pairs for the worker and apps/api.
// Public halves go to the Convex deployment env; private halves are appended to .env.local once.
const devEnvFile = join(stateDir, "dev-env.json");
const devEnvMarker = join(stateDir, "env-local-service-keys-written");
const DEV_WORKER_ISSUER = "https://sync.akatsuki.localhost";
const DEV_API_ISSUER = "https://api.akatsuki.localhost";

function serviceKey(kid) {
  const { privateKey, publicKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
  const jwk = publicKey.export({ format: "jwk" });
  return {
    kid,
    privatePem: privateKey.export({ format: "pem", type: "pkcs8" }).toString(),
    jwks: JSON.stringify({ keys: [{ ...jwk, kid, alg: "ES256", use: "sig" }] }),
  };
}

function loadDevEnv() {
  if (existsSync(devEnvFile)) return JSON.parse(readFileSync(devEnvFile, "utf8"));
  const env = {
    betterAuthSecret: randomBytes(32).toString("base64"),
    worker: serviceKey("worker-dev-1"),
    api: serviceKey("api-dev-1"),
  };
  writeFileSync(devEnvFile, JSON.stringify(env), { mode: 0o600 });
  return env;
}

function provisionDeploymentEnv(adminKey) {
  const dev = loadDevEnv();
  const vars = {
    BETTER_AUTH_SECRET: dev.betterAuthSecret,
    SITE_URL: process.env.AKATSUKI_DEV_SITE_URL ?? "http://localhost:5173",
    DEV_ORIGINS: "http://localhost:5173,http://127.0.0.1:5173,http://localhost:3000,http://127.0.0.1:3000",
    EMAIL_TRANSPORT: "console",
    AKATSUKI_BILLING: "off",
    AKATSUKI_MANAGED_AI: "off",
    WORKER_ISSUER: DEV_WORKER_ISSUER,
    API_ISSUER: DEV_API_ISSUER,
    WORKER_SERVICE_JWKS: dev.worker.jwks,
    API_SERVICE_JWKS: dev.api.jwks,
  };
  const file = join(stateDir, "convex-deployment-env");
  writeFileSync(
    file,
    `${Object.entries(vars)
      .map(([k, v]) => `${k}='${v}'`)
      .join("\n")}\n`,
    { mode: 0o600 },
  );
  const r = spawnSync("npx", ["convex", "env", "set", "--force", "--from-file", file], {
    cwd: join(root, "convex"),
    env: { ...process.env, CONVEX_SELF_HOSTED_URL: URL_, CONVEX_SELF_HOSTED_ADMIN_KEY: adminKey },
    stdio: ["ignore", "ignore", "pipe"],
    encoding: "utf8",
  });
  if (r.status !== 0) throw new Error(`convex env set failed (exit ${r.status})`);
  log(`set ${Object.keys(vars).length} local-only deployment env vars (values not shown)`);

  if (!existsSync(devEnvMarker)) {
    appendFileSync(
      join(root, ".env.local"),
      [
        "",
        "# Local-only service identities for the sync worker and apps/api (scripts/convex-dev.mjs)",
        `WORKER_ISSUER=${DEV_WORKER_ISSUER}`,
        `WORKER_SERVICE_KID=${dev.worker.kid}`,
        `WORKER_SERVICE_PRIVATE_KEY="${dev.worker.privatePem.trim()}"`,
        `API_ISSUER=${DEV_API_ISSUER}`,
        `API_SERVICE_KID=${dev.api.kid}`,
        `API_SERVICE_PRIVATE_KEY="${dev.api.privatePem.trim()}"`,
        `WORKER_SERVICE_JWKS='${dev.worker.jwks}'`,
        `CONVEX_URL=${URL_}`,
        `CONVEX_SITE_URL=http://127.0.0.1:${SITE_PORT}`,
        "",
      ].join("\n"),
      { mode: 0o600 },
    );
    writeFileSync(devEnvMarker, "1", { mode: 0o600 });
    log("appended local service-key and URL variables to .env.local (values not shown)");
  }
}

async function main() {
  mkdirSync(stateDir, { recursive: true });
  const cmd = process.argv[2] ?? "status";
  const forced = process.env.AKATSUKI_CONVEX_RUNTIME; // "docker" | "native"
  const runtime = forced ?? (dockerUsable() ? "docker" : "native");
  switch (cmd) {
    case "up": {
      if (runtime === "native" && !forced)
        log("Docker is not usable by this user; using the native convex-local-backend binary");
      const key = runtime === "docker" ? await upDocker() : await upNative();
      writeFileSync(runtimeFile, runtime);
      writeEnvLocal(key, runtime);
      provisionDeploymentEnv(key);
      log(`backend ${URL_}, HTTP actions http://127.0.0.1:${SITE_PORT}`);
      break;
    }
    case "down": {
      const used = existsSync(runtimeFile) ? readFileSync(runtimeFile, "utf8").trim() : runtime;
      if (used === "docker") execFileSync("docker", [...compose, "down"], { stdio: "inherit" });
      else if (nativeRunning()) {
        process.kill(Number(readFileSync(pidFile, "utf8")), "SIGINT");
        rmSync(pidFile);
        log("stopped native backend");
      }
      break;
    }
    case "reset": {
      const used = existsSync(runtimeFile) ? readFileSync(runtimeFile, "utf8").trim() : runtime;
      if (used === "docker") execFileSync("docker", [...compose, "down", "--volumes"], { stdio: "inherit" });
      if (nativeRunning()) process.kill(Number(readFileSync(pidFile, "utf8")), "SIGINT");
      rmSync(join(nativeDir, "data"), { recursive: true, force: true });
      rmSync(pidFile, { force: true });
      rmSync(secretFile, { force: true });
      rmSync(marker, { force: true });
      rmSync(join(stateDir, "convex-deployment-env"), { force: true });
      log(
        "local Convex data deleted; run `make convex-up` again (a new admin key is appended to .env.local)",
      );
      break;
    }
    case "status": {
      const used = existsSync(runtimeFile) ? readFileSync(runtimeFile, "utf8").trim() : "none";
      const up = await waitForVersion(1000);
      log(`runtime: ${used}; ${URL_} ${up ? "is up" : "is not answering"}`);
      break;
    }
    default:
      console.error("usage: node scripts/convex-dev.mjs up|down|reset|status");
      process.exit(2);
  }
}

await main();
