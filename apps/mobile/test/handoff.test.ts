// Mobile sign-in handoff (D-408): only a single-use code crosses `akatsuki://`; the verifier stays on the device.
import { describe, expect, it } from "vitest";
import {
  bytesToBase64Url,
  callbackCode,
  HANDOFF_CALLBACK,
  handoffUrl,
  PENDING_TTL_MS,
  parsePending,
  redeemUrl,
  toBase64Url,
} from "../src/vm/handoff.ts";

describe("handoff", () => {
  it("builds the https callback and the redeem URL on the deployment's origin", () => {
    expect(handoffUrl("https://x.convex.site/", "abc_-")).toBe(
      "https://x.convex.site/mobile/auth/handoff?challenge=abc_-",
    );
    expect(redeemUrl("https://x.convex.site")).toBe("https://x.convex.site/mobile/auth/redeem");
    expect(HANDOFF_CALLBACK).toBe("akatsuki://auth/callback");
  });

  it("encodes base64url without padding", () => {
    expect(toBase64Url("a+b/c==")).toBe("a-b_c");
    expect(bytesToBase64Url(new Uint8Array(32).fill(255))).toHaveLength(43);
  });

  it("accepts only a well-formed code", () => {
    const code = "A".repeat(43);
    expect(callbackCode(`akatsuki://auth/callback?code=${code}`)).toBe(code);
    expect(callbackCode("akatsuki://auth/callback?cookie=better-auth.session_token%3Dx")).toBeNull();
    expect(callbackCode("akatsuki://auth/callback?code=short")).toBeNull();
    expect(callbackCode("not a url")).toBeNull();
  });

  it("keeps a pending sign-in for 15 minutes, with its verifier", () => {
    const verifier = "v".repeat(43);
    const raw = JSON.stringify({ email: "ana@x.org", at: 1000, verifier });
    expect(parsePending(raw, 1000 + PENDING_TTL_MS)).toEqual({ email: "ana@x.org", at: 1000, verifier });
    expect(parsePending(raw, 1001 + PENDING_TTL_MS)).toBeNull();
    expect(parsePending(JSON.stringify({ at: 1000, verifier: "short" }), 1000)).toBeNull();
    expect(parsePending(JSON.stringify({ at: 1000, verifier }), 1000)).toEqual({ at: 1000, verifier });
    expect(parsePending("{", 0)).toBeNull();
  });
});
