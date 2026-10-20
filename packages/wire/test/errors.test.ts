// The §15 registry: fixtures/error-codes.json is the cross-language snapshot (code, HTTP status, retryable,
// client-only). The Rust registry (crates/core::wire::ErrorCode) covers every non-client code.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  ALL_ERROR_CODES,
  ERROR_CODES,
  errorBody,
  httpStatusFor,
  isErrorCode,
  parseErrorPayload,
  toErrorPayload,
  WireError,
} from "../src/errors.ts";

const fx = JSON.parse(readFileSync(new URL("../fixtures/error-codes.json", import.meta.url), "utf8")) as {
  codes: { code: string; http: number | null; retryable: boolean; clientOnly: boolean }[];
};

describe("ERROR_CODES", () => {
  it("matches the shared snapshot exactly, in order", () => {
    expect(
      ALL_ERROR_CODES.map((code) => ({
        code,
        http: ERROR_CODES[code].http,
        retryable: ERROR_CODES[code].retryable,
        clientOnly: ERROR_CODES[code].clientOnly,
      })),
    ).toEqual(fx.codes);
  });

  it("has the D-091 and D-121 additions", () => {
    expect(ERROR_CODES.not_implemented).toMatchObject({ http: 501, retryable: false });
    for (const c of ["consent_required", "ai_off", "key_missing", "ai_model_required"] as const)
      expect(ERROR_CODES[c]).toMatchObject({ http: null, clientOnly: true });
  });

  it("codes are snake_case and messages are plain sentences", () => {
    for (const code of ALL_ERROR_CODES) {
      expect(code).toMatch(/^[a-z]+(_[a-z]+)*$/);
      expect(ERROR_CODES[code].message).toMatch(/^[A-Z].*\.$/);
    }
  });

  it("maps codes without a status to 400, like Rust", () => {
    expect(httpStatusFor("draft_deleted")).toBe(400);
    expect(httpStatusFor("provider_quota")).toBe(507);
  });
});

describe("envelopes", () => {
  it("builds and parses §15 bodies", () => {
    const body = errorBody("rate_limited", { retryAfterMs: 1499.6 });
    expect(body).toEqual({
      error: {
        code: "rate_limited",
        message: "Too many requests. Try again shortly.",
        retryable: true,
        retryAfterMs: 1500,
      },
    });
    expect(parseErrorPayload(body)).toEqual(body.error);
    expect(parseErrorPayload(body.error)).toEqual(body.error);
    expect(
      parseErrorPayload({
        error: { code: "draft_version_conflict", message: "x", retryable: false, version: 7 },
      }),
    ).toMatchObject({ version: 7 });
    expect(parseErrorPayload({ nope: 1 })).toBeNull();
    expect(parseErrorPayload("text")).toBeNull();
  });

  it("fills defaults for partial payloads and keeps unknown codes", () => {
    expect(parseErrorPayload({ code: "internal" })).toEqual({
      code: "internal",
      message: "Something went wrong.",
      retryable: true,
    });
    const future = parseErrorPayload({
      error: { code: "brand_new_code", message: "Later.", retryable: false },
    });
    expect(future?.code).toBe("brand_new_code");
    expect(isErrorCode("brand_new_code")).toBe(false);
    expect(isErrorCode("not_found")).toBe(true);
    expect(isErrorCode("toString")).toBe(false);
  });

  it("normalises thrown values", () => {
    expect(toErrorPayload(WireError.of("not_found"))).toEqual({
      code: "not_found",
      message: "Not found.",
      retryable: false,
    });
    // A ConvexError carries its payload in `data`.
    expect(toErrorPayload({ data: { code: "invalid_action", message: "No.", retryable: false } }).code).toBe(
      "invalid_action",
    );
    // AiFeatureError-like objects.
    expect(toErrorPayload({ code: "key_missing", message: "Key not set on this device." }).code).toBe(
      "key_missing",
    );
    expect(toErrorPayload(new Error("boom"))).toEqual(errorBody("internal").error);
  });
});
