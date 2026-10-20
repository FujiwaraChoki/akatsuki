// Managed dictation cap (§13.4: 120 min/day per user, D-343): ee.service.aiPrecheck checks the daily
// transcription bucket, ee.service.aiRecordUsage takes the real audio length from it.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../_generated/api";
import { TRANSCRIPTION_SECONDS_PER_DAY } from "../ee/service";
import { asApi, entitledUser, makeTest, setEnv } from "./setup.helpers";

const precheck = (audioSeconds?: number, userId = "u1") => ({
  userId,
  feature: "dictation" as const,
  tier: "transcription" as const,
  estCostMicros: 100,
  ...(audioSeconds === undefined ? {} : { audioSeconds }),
});
const usage = (audioSeconds: number, userId = "u1") => ({
  userId,
  feature: "dictation" as const,
  model: "gpt-4o-transcribe",
  inputTokens: 0,
  outputTokens: 0,
  audioSeconds,
  costMicros: Math.ceil(audioSeconds / 60) * 6000,
});

describe("transcription daily cap", () => {
  beforeEach(() => {
    setEnv();
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-20T10:00:00Z"));
  });
  afterEach(() => vi.useRealTimers());

  it("allows 120 minutes per UTC day, then refuses dictation until midnight", async () => {
    expect(TRANSCRIPTION_SECONDS_PER_DAY).toBe(7200);
    const t = makeTest();
    await entitledUser(t, "u1");
    const svc = asApi(t);

    // 11 recordings of 10 minutes and one of 9: 7,140 s used, 60 s left.
    for (let i = 0; i < 11; i++) {
      expect((await svc.mutation(api.ee.service.aiPrecheck, precheck(600))).ok).toBe(true);
      await svc.mutation(api.ee.service.aiRecordUsage, usage(600));
    }
    await svc.mutation(api.ee.service.aiRecordUsage, usage(540));
    expect((await svc.mutation(api.ee.service.aiPrecheck, precheck(60))).ok).toBe(true);

    const refused = await svc.mutation(api.ee.service.aiPrecheck, precheck(61));
    expect(refused).toMatchObject({ ok: false, code: "rate_limited", limit: "transcription_day" });
    const midnight = Date.parse("2026-10-21T00:00:00Z");
    expect(refused.ok ? 0 : refused.retryAfterMs).toBe(midnight - Date.now());

    // A recording longer than its estimate is counted up to the cap, not into tomorrow.
    await svc.mutation(api.ee.service.aiRecordUsage, usage(300));
    expect((await svc.mutation(api.ee.service.aiPrecheck, precheck())).ok).toBe(false);

    // Text features are not affected by the transcription cap.
    const text = await svc.mutation(api.ee.service.aiPrecheck, {
      userId: "u1",
      feature: "summarize",
      tier: "default",
      estCostMicros: 100,
    });
    expect(text.ok).toBe(true);

    // Other users have their own bucket.
    await entitledUser(t, "u2");
    expect((await svc.mutation(api.ee.service.aiPrecheck, precheck(600, "u2"))).ok).toBe(true);

    // The next UTC day starts with a full bucket.
    vi.setSystemTime(new Date("2026-10-21T00:00:01Z"));
    expect((await svc.mutation(api.ee.service.aiPrecheck, precheck(600))).ok).toBe(true);
  });

  it("still checks entitlement and the budget first", async () => {
    const t = makeTest();
    const svc = asApi(t);
    expect(await svc.mutation(api.ee.service.aiPrecheck, precheck(10, "nobody"))).toEqual({
      ok: false,
      code: "not_entitled",
    });
  });
});
