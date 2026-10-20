#!/usr/bin/env node
// Local end-to-end run without Docker (DECISIONS D-382): the real Convex functions on the local
// self-hosted backend, the real sync worker, and the fake IMAP + SMTP servers of crates/providers.
//
//   node scripts/e2e-local.mjs            API flow (below), then stop everything
//   node scripts/e2e-local.mjs --web      API flow, then the web app's Playwright local suites (e2e/local)
//   node scripts/e2e-local.mjs --serve    start fake mail + worker and keep them running (Ctrl-C stops)
//   flags: --skip-deploy (functions already pushed), --skip-build (binaries already built)
//   make e2e  /  make e2e-web
//
// API flow: sign in a fresh test user by email code (EMAIL_TRANSPORT=console on the harness; the code is read
// from the deployment's function logs for that address only) → connect the fake IMAP account through
// POST /v1/connect/imap → threads appear in Convex → GET /v1/messages/{id} returns sanitized HTML → an archive
// op moves the message on the fake server, undo moves it back → PUT /v1/drafts + POST /v1/send reaches the
// fake SMTP server and the Sent folder → a newly delivered message reaches Convex → the account is removed.
//
// Secrets: this script never reads .env.local. The worker is started through `node --env-file` (Node reads the
// file, as the Convex CLI does with `--env-file`), so its variables reach the worker process only. The worker's
// symmetric keys for this harness are random and kept in .convex-dev/e2e/worker-keys.json (0600, gitignored).
// Nothing secret is printed: worker logs are content-free JSON and go to .convex-dev/e2e/run-*/worker.log.
import { spawn, spawnSync } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { existsSync, mkdirSync, openSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const envLocal = join(root, ".env.local");

// ───────────── helpers ─────────────
function log(msg) {
  console.info(`[e2e] ${msg}`);
}

function fail(msg) {
  throw new Error(msg);
}

function check(cond, msg) {
  if (!cond) fail(`check failed: ${msg}`);
  log(`ok  ${msg}`);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitFor(label, fn, { timeoutMs = 60_000, intervalMs = 500 } = {}) {
  const start = Date.now();
  let last;
  while (Date.now() - start < timeoutMs) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await sleep(intervalMs);
  }
  fail(`timed out after ${timeoutMs} ms: ${label}${last ? ` (last error: ${last.message})` : ""}`);
}

function freePort(preferred) {
  return new Promise((res) => {
    const s = createServer();
    s.once("error", () => {
      const t = createServer();
      t.listen(0, "127.0.0.1", () => {
        const { port } = t.address();
        t.close(() => res(port));
      });
    });
    s.listen(preferred, "127.0.0.1", () => s.close(() => res(preferred)));
  });
}

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: root, stdio: "inherit", ...opts });
  if (r.status !== 0) fail(`${cmd} ${args.join(" ")} failed (exit ${r.status})`);
}

const children = [];
function stopAll() {
  for (const c of children.splice(0).reverse()) {
    if (c.exitCode === null && c.signalCode === null) c.kill("SIGTERM");
  }
}

// ───────────── configuration (no secrets) ─────────────
const CONVEX_URL = `http://127.0.0.1:${process.env.AKATSUKI_CONVEX_PORT ?? 3410}`;
const SITE_URL = `http://127.0.0.1:${process.env.AKATSUKI_CONVEX_SITE_PORT ?? 3411}`;
const ORIGIN = "http://localhost:5173";

function workerKeys(stateDir) {
  const file = join(stateDir, "worker-keys.json");
  if (existsSync(file)) return JSON.parse(readFileSync(file, "utf8"));
  const k = () => randomBytes(32).toString("base64");
  const keys = {
    TOKEN_ENC_KEYS: `e2e1:${k()}`,
    STATE_KEY: k(),
    SPOOL_KEY: k(),
    LINK_SIGNING_KEY: k(),
    IMG_PROXY_KEY: k(),
    READ_RECEIPT_KEY: k(),
  };
  writeFileSync(file, JSON.stringify(keys), { mode: 0o600 });
  return keys;
}

// ───────────── Convex and Better Auth over HTTP ─────────────
async function convex(kind, path, args, jwt) {
  const res = await fetch(`${CONVEX_URL}/api/${kind}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...(jwt ? { authorization: `Bearer ${jwt}` } : {}) },
    body: JSON.stringify({ path, args, format: "json" }),
  });
  const body = await res.json();
  if (body.status !== "success") {
    const code = body.errorData?.code ?? "";
    fail(`${kind} ${path} failed: ${code} ${body.errorMessage ?? res.status}`.trim());
  }
  return body.value;
}
const query = (p, a, jwt) => convex("query", p, a, jwt);
const mutation = (p, a, jwt) => convex("mutation", p, a, jwt);

async function auth(path, body, bearer) {
  return fetch(`${SITE_URL}/api/auth${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: {
      origin: ORIGIN,
      ...(body === undefined ? {} : { "content-type": "application/json" }),
      ...(bearer ? { authorization: `Bearer ${bearer}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

/** The 6-digit code the console transport logged for `email` (this address only; nothing is printed). */
function readOtp(email, timeoutMs = 45_000) {
  return new Promise((res, rej) => {
    const child = spawn(
      "pnpm",
      ["exec", "convex", "logs", "--history", "100", "--env-file", "../.env.local"],
      {
        cwd: join(root, "convex"),
        stdio: ["ignore", "pipe", "ignore"],
      },
    );
    let out = "";
    const timer = setTimeout(() => {
      child.kill();
      rej(new Error("no sign-in code in the function logs"));
    }, timeoutMs);
    child.stdout.on("data", (d) => {
      out += d.toString();
      const at = out.lastIndexOf(`to=${email}`);
      if (at === -1) return;
      const m = /\b(\d{6})\b/.exec(out.slice(at + email.length + 3));
      if (!m) return;
      clearTimeout(timer);
      child.kill();
      res(m[1]);
    });
  });
}

async function signIn(email) {
  const otp = readOtp(email);
  await sleep(1500); // let `convex logs` attach before the email is logged
  const sent = await auth("/email-otp/send-verification-otp", { email, type: "sign-in" });
  if (!sent.ok) fail(`send-verification-otp: HTTP ${sent.status}`);
  const code = await otp;
  const signed = await auth("/sign-in/email-otp", { email, otp: code });
  if (!signed.ok) fail(`sign-in/email-otp: HTTP ${signed.status}`);
  const session = signed.headers.get("set-auth-token") ?? (await signed.json()).token;
  if (!session) fail("no session token");
  const jwtRes = await auth("/convex/token", undefined, session);
  if (!jwtRes.ok) fail(`convex/token: HTTP ${jwtRes.status}`);
  const { token } = await jwtRes.json();
  if (!token) fail("no JWT");
  return { session, jwt: token };
}

// ───────────── processes ─────────────
function startFakeMail(runDir) {
  const stateFile = join(runDir, "fake-mail.json");
  const caFile = join(runDir, "fake-mail-roots.pem");
  const out = openSync(join(runDir, "fake-mail.log"), "a");
  const child = spawn(
    join(root, "target/debug/akatsuki-fake-mail"),
    ["--state", stateFile, "--ca-file", caFile],
    { stdio: ["ignore", out, out] },
  );
  children.push(child);
  return { child, stateFile, caFile };
}

function startWorker(runDir, extra) {
  const out = openSync(join(runDir, "worker.log"), "a");
  const node = process.execPath;
  const args = [
    ...(existsSync(envLocal) ? [`--env-file=${envLocal}`] : []),
    fileURLToPath(import.meta.url),
    "__exec",
    join(root, "target/debug/akatsuki-sync"),
  ];
  // Variables given here win over .env.local (Node's --env-file never overrides the environment).
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, RUST_LOG: "info", ...extra };
  const child = spawn(node, args, { stdio: ["ignore", out, out], env });
  children.push(child);
  return child;
}

async function worker(base, method, path, jwt, body) {
  const res = await fetch(`${base}${path}`, {
    method,
    headers: {
      authorization: `Bearer ${jwt}`,
      origin: ORIGIN,
      ...(body ? { "content-type": "application/json" } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  return res;
}

// ───────────── the API flow ─────────────
async function apiFlow(ctx) {
  const { syncUrl, fake } = ctx;
  const fakeState = async () => (await fetch(`${fake.controlUrl}/state`)).json();
  const email = `e2e-${Date.now()}@example.com`;

  log(`signing in ${email} with an email code`);
  const { session, jwt } = await signIn(email);
  check(jwt.split(".").length === 3, "email-code sign-in returned a Convex JWT");
  await mutation("me:bootstrap", { timezone: "UTC" }, jwt);
  const me = await query("me:get", {}, jwt);
  check(me?.entitlement?.hostedSync === true, "the user is entitled (AKATSUKI_BILLING=off on the harness)");

  log("connecting the fake IMAP account through the worker");
  const connect = await worker(syncUrl, "POST", "/v1/connect/imap", jwt, {
    email: fake.email,
    username: fake.username,
    password: fake.password,
    imap: { host: fake.host, port: fake.imapPort, tls: "implicit" },
    smtp: { host: fake.host, port: fake.smtpPort, tls: "implicit" },
  });
  const connected = await connect.json();
  check(
    connect.status === 201 && connected.accountId,
    `POST /v1/connect/imap → 201 (got ${connect.status}${connected.error ? ` ${connected.error.code}` : ""})`,
  );
  const accountId = connected.accountId;
  ctx.cleanup.push(async () => {
    const accounts = await query("accounts:list", {}, jwt).catch(() => []);
    if (accounts.some((a) => a._id === accountId)) await mutation("accounts:remove", { accountId }, jwt);
  });

  const inbox = async () =>
    (
      await query(
        "mail:listThreads",
        { view: { labelKey: "sys:inbox", accountId }, paginationOpts: { numItems: 50, cursor: null } },
        jwt,
      )
    ).page;
  const threads = await waitFor("3 inbox threads in Convex", async () => {
    const page = await inbox();
    return page.length >= 3 ? page : null;
  });
  const bySubject = (s) => threads.find((t) => t.subject === s);
  check(threads.length === 3, `INBOX synced to Convex as 3 threads (4 messages)`);
  check(bySubject("Lunch on Friday?")?.messageCount === 2, "the reply joined its thread (References)");
  const hostile = bySubject("Quarterly report (HTML)");
  const welcome = bySubject("Welcome to the e2e inbox");
  check(hostile && welcome, "thread subjects match the fake mailbox");

  // Body through the worker, sanitized.
  const detail = await query("mail:getThread", { threadId: hostile._id }, jwt);
  const messageId = detail.messages[0]._id;
  const bodyRes = await worker(syncUrl, "GET", `/v1/messages/${messageId}`, jwt);
  check(bodyRes.status === 200, `GET /v1/messages/{id} → 200 (got ${bodyRes.status})`);
  check(
    /no-store/.test(bodyRes.headers.get("cache-control") ?? ""),
    "body response is Cache-Control: no-store",
  );
  const body = await bodyRes.json();
  const html = body.html ?? "";
  check(
    html.includes("Quarterly report") && html.includes("read it online"),
    "sanitized HTML keeps the content",
  );
  for (const bad of ["<script", "onload", "onclick", "javascript:", "<form", "<iframe", "<input"]) {
    check(!html.toLowerCase().includes(bad), `sanitized HTML has no ${bad}`);
  }
  check(!/\ssrc="https?:/i.test(html), "no remote image loads directly");
  check(
    html.includes(`data-akatsuki-src="${syncUrl}/v1/img/`),
    "remote images point at the signed image proxy",
  );
  check(body.remoteImageCount >= 1, `remote images counted (${body.remoteImageCount})`);
  check(body.trackersRemoved >= 1, `tracking pixel removed (${body.trackersRemoved})`);
  check(typeof body.sanitizerVersion === "number", "MessageBody carries sanitizerVersion");

  const lunch = bySubject("Lunch on Friday?");
  const ndjson = await worker(syncUrl, "GET", `/v1/threads/${lunch._id}/bodies`, jwt);
  const lines = (await ndjson.text()).trim().split("\n").filter(Boolean);
  check(ndjson.status === 200 && lines.length === 2, "GET /v1/threads/{id}/bodies streams both messages");

  // Archive: a move to the account's archive folder (folder provider), then undo.
  const labels = await query("labels:list", { accountId }, jwt);
  const archive = labels.find((l) => l.role === "archive");
  check(archive, "the Archive folder (\\Archive) is mirrored as a label with role archive");
  const archiveOp = `e2e-archive-${randomUUID()}`;
  await mutation(
    "mail:apply",
    { clientOpId: archiveOp, threadIds: [welcome._id], action: { kind: "move", to: `l:${archive._id}` } },
    jwt,
  );
  const where = (s, subject) =>
    (s.mailboxes.INBOX ?? []).some((m) => m.subject === subject)
      ? "INBOX"
      : (s.mailboxes.Archive ?? []).some((m) => m.subject === subject)
        ? "Archive"
        : "elsewhere";
  await waitFor(
    "the welcome message moves to Archive on the fake server",
    async () => where(await fakeState(), "Welcome to the e2e inbox") === "Archive",
  );
  check(true, "archive op reached the IMAP server (UID MOVE to Archive)");
  await waitFor("the archive op is done in Convex", async () => {
    const t = await query("mail:getThread", { threadId: welcome._id }, jwt);
    return t.pendingOps.length === 0 && !t.thread.labelKeys.includes("sys:inbox");
  });
  check(!(await inbox()).some((t) => t._id === welcome._id), "the archived thread left the inbox view");

  const undo = await mutation(
    "mail:undo",
    { clientOpId: `e2e-undo-${randomUUID()}`, undoClientOpId: archiveOp },
    jwt,
  );
  check(undo.mode === "inverse" || undo.mode === "cancelled", `undo accepted (${undo.mode})`);
  await waitFor(
    "undo moves the message back to INBOX",
    async () => where(await fakeState(), "Welcome to the e2e inbox") === "INBOX",
  );
  await waitFor("the thread is back in the inbox view", async () =>
    (await inbox()).some((t) => t.subject === "Welcome to the e2e inbox"),
  );
  check(true, "undo reached the IMAP server and Convex");

  // Send: save the draft through the worker, then send with no undo delay.
  const subject = `e2e send ${Date.now()}`;
  const clientDraftId = `e2e-draft-${randomUUID()}`;
  const deviceId = "e2e-script";
  const draft = {
    accountId,
    identityEmail: fake.email,
    to: [{ name: "Carol Example", email: "carol@example.com" }],
    cc: [],
    bcc: [],
    subject,
    html: "<p>Hi <b>Carol</b>, see you Friday.</p><script>alert(1)</script>",
    text: "Hi Carol, see you Friday.",
    attachments: [],
  };
  const put = await worker(syncUrl, "PUT", `/v1/drafts/${clientDraftId}`, jwt, {
    baseVersion: 0,
    draft,
    deviceId,
  });
  const saved = await put.json();
  check(
    put.status === 200 && saved.version >= 1,
    `PUT /v1/drafts → 200 (got ${put.status}${saved.error ? ` ${saved.error.code}` : ""})`,
  );
  await waitFor("the draft is appended to Drafts on the fake server", async () =>
    ((await fakeState()).mailboxes.Drafts ?? []).some((m) => m.subject === subject),
  );
  check(true, "draft saved to the IMAP Drafts folder");
  const sendRes = await worker(syncUrl, "POST", "/v1/send", jwt, {
    clientSendId: `e2e-send-${randomUUID()}`,
    clientDraftId,
    baseVersion: saved.version,
    undoSeconds: 0,
    deviceId,
  });
  const accepted = await sendRes.json();
  check(
    sendRes.status === 202 && accepted.sendId,
    `POST /v1/send → 202 (got ${sendRes.status}${accepted.error ? ` ${accepted.error.code}` : ""})`,
  );
  const delivered = await waitFor("the message reaches the fake SMTP server", async () =>
    (await fakeState()).delivered.find((d) => d.subject === subject),
  );
  check(delivered.rcpts.includes("carol@example.com"), "SMTP RCPT TO carol@example.com");
  check(delivered.from === fake.email, "SMTP MAIL FROM is the account address");
  check(/@/.test(delivered.messageId), "the sent message has a Message-ID");
  check(delivered.html && !delivered.hasScript, "the sent HTML part was re-sanitized (no <script>)");
  check(!delivered.hasBccHeader, "no Bcc header on the wire");
  await waitFor("the sent message is filed in Sent and the draft removed", async () => {
    const s = await fakeState();
    return (
      (s.mailboxes.Sent ?? []).some((m) => m.subject === subject) &&
      !(s.mailboxes.Drafts ?? []).some((m) => m.subject === subject)
    );
  });
  check(true, "Sent copy appended, draft removed");
  await waitFor("the scheduled send leaves the active states", async () => {
    const open = await query("sends:list", {}, jwt);
    return !open.some((s) => s._id === accepted.sendId);
  });
  check(true, "the send is complete in Convex");

  // New mail on the server reaches Convex (IMAP IDLE, or the poll).
  const fresh = `e2e new mail ${Date.now()}`;
  await fetch(`${fake.controlUrl}/deliver`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ from: "Erin Example <erin@example.com>", subject: fresh, text: "Fresh mail." }),
  });
  await waitFor(
    "a newly delivered message reaches Convex",
    async () => (await inbox()).some((t) => t.subject === fresh),
    { timeoutMs: 120_000, intervalMs: 1000 },
  );
  check(true, "new mail on the server synced to Convex");

  // Clean up: remove the account (the worker purges it) and sign out.
  await mutation("accounts:remove", { accountId }, jwt);
  await waitFor("the account is purged", async () => {
    const accounts = await query("accounts:list", {}, jwt);
    return !accounts.some((a) => a._id === accountId);
  });
  check(true, "account removed and purged");
  const out = await auth("/sign-out", {}, session);
  check(out.ok, "signed out");
}

// ───────────── main ─────────────
async function main() {
  const argv = new Set(process.argv.slice(2));
  const stateDir = join(root, ".convex-dev", "e2e");
  const runDir = join(stateDir, `run-${new Date().toISOString().replaceAll(/[:.]/g, "-")}`);
  mkdirSync(runDir, { recursive: true, mode: 0o700 });
  // Keep the logs of the last five runs.
  const runs = readdirSync(stateDir)
    .filter((d) => d.startsWith("run-"))
    .sort();
  for (const old of runs.slice(0, -5)) rmSync(join(stateDir, old), { recursive: true, force: true });

  for (const sig of ["SIGINT", "SIGTERM"]) {
    process.on(sig, () => {
      stopAll();
      process.exit(130);
    });
  }

  try {
    log("local Convex backend (scripts/convex-dev.mjs up)");
    run(process.execPath, [join(root, "scripts/convex-dev.mjs"), "up"]);
    if (!argv.has("--skip-deploy")) {
      log("pushing Convex functions (pnpm convex:dev:deploy)");
      run("pnpm", ["convex:dev:deploy"]);
    }
    if (!argv.has("--skip-build")) {
      log("building the worker and the fake mail servers");
      run("cargo", ["build", "--locked", "-p", "akatsuki-sync", "--features", "e2e-fakes", "--bins"]);
    }

    const fakeProc = startFakeMail(runDir);
    await waitFor("fake mail servers", async () => existsSync(fakeProc.stateFile), { timeoutMs: 15_000 });
    const fake = JSON.parse(readFileSync(fakeProc.stateFile, "utf8"));
    log(`fake IMAP 127.0.0.1:${fake.imapPort}, SMTP 127.0.0.1:${fake.smtpPort} (implicit TLS)`);

    const port = await freePort(Number(process.env.AKATSUKI_E2E_SYNC_PORT ?? 18787));
    const syncUrl = `http://127.0.0.1:${port}`;
    const keys = workerKeys(stateDir);
    startWorker(runDir, {
      ...keys,
      BIND_ADDR: `127.0.0.1:${port}`,
      PUBLIC_BASE_URL: syncUrl,
      CORS_ORIGINS: "http://localhost:5173,http://127.0.0.1:5173",
      SYNC_PUSH: "on",
      MAIL_CA_FILE: fakeProc.caFile,
      // The fake IMAP/SMTP servers listen on loopback, which the hosted worker never contacts (D-398).
      MAIL_ALLOW_PRIVATE_HOSTS: "1",
      SPOOL_DIR: join(runDir, "spool"),
      CONVEX_URL,
      CONVEX_SITE_URL: SITE_URL,
    });
    await waitFor(
      "worker /readyz (Convex subscription live)",
      async () => (await fetch(`${syncUrl}/readyz`)).ok,
      {
        timeoutMs: 60_000,
      },
    );
    log(`sync worker ${syncUrl} (logs: ${join(runDir, "worker.log")})`);

    const ctx = { syncUrl, fake, runDir, cleanup: [] };
    if (argv.has("--serve")) {
      writeFileSync(
        join(stateDir, "serve.json"),
        JSON.stringify({ syncUrl, fakeState: fakeProc.stateFile }),
        {
          mode: 0o600,
        },
      );
      log("serving; Ctrl-C stops");
      await new Promise(() => {});
    }
    try {
      await apiFlow(ctx);
    } finally {
      for (const f of ctx.cleanup) await f().catch(() => {});
    }
    log("API flow passed");

    if (argv.has("--web")) {
      // Every local spec: mail.spec.ts (onboarding, inbox, archive/undo, removal) and auth.spec.ts (sign-in,
      // worker error copy, theme round trip, sign-out). Sign-ins on the loopback harness carry no client IP,
      // so Better Auth's per-IP limits do not apply to them (D-401); each spec uses its own address.
      log("web app: Playwright local suites against the harness, the worker and the fake mailbox");
      run("pnpm", ["--filter", "@akatsuki/web", "exec", "playwright", "test", "e2e/local"], {
        env: {
          PATH: process.env.PATH,
          HOME: process.env.HOME,
          AKATSUKI_E2E_LOCAL: "1",
          AKATSUKI_E2E_SYNC_URL: syncUrl,
          AKATSUKI_E2E_FAKE_MAIL_STATE: fakeProc.stateFile,
        },
      });
      log("web suite passed");
    }
  } finally {
    stopAll();
  }
}

// ───────────── entry ─────────────
// `__exec` runs a binary with the variables of .env.local; it is only ever started through `node --env-file`.
if (process.argv[2] === "__exec") {
  const [bin, ...args] = process.argv.slice(3);
  const child = spawn(bin, args, { stdio: "inherit", env: process.env });
  for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => child.kill(sig));
  child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
} else {
  try {
    await main();
  } catch (e) {
    console.error(`[e2e] FAILED: ${e.message}`);
    process.exit(1);
  }
}
