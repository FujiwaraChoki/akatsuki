// node --test infra/scripts/  (run by CI's infra job)
import assert from "node:assert/strict";
import { createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { generate, jwks, main, thumbprint, WORKER_SECRET_NAMES } from "./gen-service-keys.mjs";

/** Minimal parser for the single-quoted NAME='value' lines the script writes. */
function parseEnv(text) {
  const out = {};
  for (const raw of text.split("\n")) {
    if (!raw || raw.startsWith("#")) continue;
    const m = /^([A-Z0-9_]+)='([^']*)'$/.exec(raw);
    assert.ok(m, `unexpected env line shape: ${raw.slice(0, 20)}`);
    out[m[1]] = m[2];
  }
  return out;
}

const withTmp = (fn) => {
  const dir = mkdtempSync(join(tmpdir(), "akatsuki-keys-"));
  try {
    return fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
};

test("worker and api keys sign ES256 and verify against their published JWKS", () => {
  const { files } = generate();
  const sync = parseEnv(files["sync.env.part"]);
  const api = parseEnv(files["api.env.part"]);
  const convex = parseEnv(files["convex.env.part"]);
  for (const [env, prefix, jwksName] of [
    [sync, "WORKER", "WORKER_SERVICE_JWKS"],
    [api, "API", "API_SERVICE_JWKS"],
  ]) {
    // Services turn the literal \n back into newlines (apps/sync config.rs, apps/api config.ts).
    const pem = env[`${prefix}_SERVICE_PRIVATE_KEY`].replaceAll("\\n", "\n");
    const priv = createPrivateKey(pem);
    assert.equal(priv.asymmetricKeyType, "ec");
    assert.equal(priv.asymmetricKeyDetails.namedCurve, "prime256v1");
    const set = JSON.parse(convex[jwksName]);
    const jwk = set.keys.find((k) => k.kid === env[`${prefix}_SERVICE_KID`]);
    assert.ok(jwk, "kid is published");
    assert.equal(jwk.alg, "ES256");
    assert.equal(jwk.d, undefined, "no private component in the JWKS");
    const data = Buffer.from("header.payload");
    const sig = sign("sha256", data, { key: priv, dsaEncoding: "ieee-p1363" });
    assert.ok(
      verify(
        "sha256",
        data,
        { key: createPublicKey({ key: jwk, format: "jwk" }), dsaEncoding: "ieee-p1363" },
        sig,
      ),
    );
  }
  assert.equal(api.WORKER_SERVICE_JWKS, convex.WORKER_SERVICE_JWKS, "apps/api verifies worker tokens");
  assert.equal(convex.WORKER_ISSUER, "https://sync.akatsuki.vc");
  assert.equal(convex.API_ISSUER, "https://api.akatsuki.vc");
  assert.ok(!files["convex.env.part"].includes("PRIVATE KEY"));
});

test("worker secrets are 32-byte base64 keys in the shape apps/sync parses", () => {
  const sync = parseEnv(generate({ withWorkerSecrets: true }).files["sync.env.part"]);
  assert.match(sync.TOKEN_ENC_KEYS, /^k1:[A-Za-z0-9+/]{43}=$/);
  for (const name of WORKER_SECRET_NAMES) {
    assert.equal(Buffer.from(sync[name], "base64").length, 32, name);
  }
  const noSecrets = parseEnv(generate().files["sync.env.part"]);
  assert.equal(noSecrets.TOKEN_ENC_KEYS, undefined);
});

test("rotation publishes the previous public key next to the new one", () => {
  const first = generate();
  const previous = JSON.parse(first.files["service-keys.json"]);
  const second = generate({ previous });
  const kids = JSON.parse(second.workerJwks).keys.map((k) => k.kid);
  assert.deepEqual(kids, [second.worker.kid, first.worker.kid]);
  assert.equal(JSON.parse(jwks(first.api.publicJwk, first.api.publicJwk)).keys.length, 1, "dedupes by kid");
});

test("main writes 0600 files, never prints private material and refuses to overwrite", () => {
  withTmp((dir) => {
    const out = join(dir, "keys");
    const lines = [];
    main(["--out", out, "--with-worker-secrets"], (l) => lines.push(l));
    const printed = lines.join("\n");
    assert.ok(!printed.includes("PRIVATE"), "no PEM on stdout");
    for (const name of ["service-keys.json", "sync.env.part", "api.env.part", "convex.env.part"]) {
      assert.equal(statSync(join(out, name)).mode & 0o777, 0o600, name);
    }
    assert.equal(statSync(out).mode & 0o777, 0o700);
    const stored = JSON.parse(readFileSync(join(out, "service-keys.json"), "utf8"));
    assert.ok(printed.includes(thumbprint(stored.worker.publicJwk)));
    for (const secret of Object.values(stored.workerSecrets)) assert.ok(!printed.includes(secret));
    assert.throws(() => main(["--out", out], () => {}), /refusing to overwrite/);
  });
});

test("unknown arguments are rejected", () => {
  assert.throws(() => main(["--nope"], () => {}), /unknown argument/);
});
