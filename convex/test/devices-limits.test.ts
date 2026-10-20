// Devices and send tombstones (D-403): a push token belongs to one row, devices are capped per user, Expo's
// DeviceNotRegistered drops the token, and sends.cancel validates and rate-limits the rows it creates.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api, internal } from "../_generated/api";
import { MAX_DEVICES } from "../devices";
import { unregisteredTokens } from "../notify";
import {
  asUser,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  seedThread,
  setEnv,
  type T,
} from "./setup.helpers";

const reg = (t: T, userId: string, deviceId: string, expoPushToken?: string) =>
  asUser(t, userId).mutation(api.devices.register, {
    deviceId,
    kind: "mobile",
    name: "Phone",
    appVersion: "0.1.0",
    ...(expoPushToken ? { expoPushToken } : {}),
  });

const rows = (t: T) => t.run(async (ctx) => await ctx.db.query("devices").collect());

describe("devices", () => {
  beforeEach(() => setEnv());
  afterEach(() => vi.unstubAllGlobals());

  it("a push token moves to whoever registers it last (shared or handed-over phone)", async () => {
    const t = makeTest();
    const token = "ExponentPushToken[shared-phone]";
    await reg(t, "u-dev-a", "device-aaaaaaaa", token);
    // A signs out without reaching the server; B signs in on the same phone.
    await reg(t, "u-dev-b", "device-aaaaaaaa", token);
    const all = await rows(t);
    expect(all.find((d) => d.userId === "u-dev-a")?.expoPushToken).toBeUndefined();
    expect(all.find((d) => d.userId === "u-dev-b")?.expoPushToken).toBe(token);
    const a = await t.query(internal.notify.pushTargets, { userId: "u-dev-a", threadIds: [] });
    expect(a.tokens).toEqual([]);
  });

  it("sign-out removes this device's row", async () => {
    const t = makeTest();
    await reg(t, "u-dev-out", "device-out-1", "ExponentPushToken[out]");
    await asUser(t, "u-dev-out").mutation(api.devices.remove, { deviceId: "device-out-1" });
    expect(await rows(t)).toEqual([]);
  });

  it("keeps at most MAX_DEVICES rows per user, evicting the least recently seen", async () => {
    const t = makeTest();
    await t.run(async (ctx) => {
      for (let i = 0; i < MAX_DEVICES; i++)
        await ctx.db.insert("devices", {
          userId: "u-dev-cap",
          deviceId: `seeded-${String(i).padStart(4, "0")}`,
          kind: "web",
          name: "Browser",
          appVersion: "1",
          lastSeenAt: 1000 + i,
        });
    });
    await reg(t, "u-dev-cap", "device-newest");
    const mine = (await rows(t)).filter((d) => d.userId === "u-dev-cap");
    expect(mine).toHaveLength(MAX_DEVICES);
    expect(mine.some((d) => d.deviceId === "seeded-0000")).toBe(false);
    expect(mine.some((d) => d.deviceId === "device-newest")).toBe(true);
  });

  it("rate-limits new device rows per user, not updates of a known one", async () => {
    const t = makeTest();
    for (let i = 0; i < 20; i++) await reg(t, "u-dev-rate", `device-rate-${String(i).padStart(3, "0")}`);
    await expect(reg(t, "u-dev-rate", "device-rate-999")).rejects.toThrow(/rate_limited/);
    await reg(t, "u-dev-rate", "device-rate-000");
  });

  it("rejects oversized push tokens", async () => {
    const t = makeTest();
    await expect(reg(t, "u-dev-bad", "device-bad-1", "x".repeat(300))).rejects.toThrow(/invalid_argument/);
  });

  it("drops tokens Expo reports as DeviceNotRegistered", async () => {
    const t = makeTest();
    await entitledUser(t, "u-dev-gone");
    const { accountId } = await connectGmail(t, "u-dev-gone");
    const threadId = await seedThread(t, accountId, "thr-gone", [message()]);
    await reg(t, "u-dev-gone", "device-gone-1", "ExponentPushToken[gone]");
    await reg(t, "u-dev-gone", "device-kept-1", "ExponentPushToken[kept]");
    vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
      const sent = JSON.parse(String(init.body)) as { to: string }[];
      return new Response(
        JSON.stringify({
          data: sent.map((m) =>
            m.to.includes("gone")
              ? { status: "error", message: "not registered", details: { error: "DeviceNotRegistered" } }
              : { status: "ok", id: "ticket" },
          ),
        }),
        { status: 200 },
      );
    });
    await t.action(internal.notify.newMail, { userId: "u-dev-gone", threadIds: [threadId] });
    const tokens = (await rows(t)).map((d) => d.expoPushToken);
    expect(tokens).toContain("ExponentPushToken[kept]");
    expect(tokens).not.toContain("ExponentPushToken[gone]");
  });

  it("parses Expo push tickets in message order", () => {
    const batch = [{ to: "a" }, { to: "b" }, { to: "c" }];
    expect(
      unregisteredTokens(batch, {
        data: [
          { status: "ok" },
          { status: "error", details: { error: "DeviceNotRegistered" } },
          { status: "error", details: { error: "MessageRateExceeded" } },
        ],
      }),
    ).toEqual(["b"]);
    expect(unregisteredTokens(batch, null)).toEqual([]);
    expect(unregisteredTokens(batch, { errors: [{}] })).toEqual([]);
  });
});

describe("sends.cancel tombstones", () => {
  beforeEach(() => setEnv());

  it("validates the client send id like the worker does", async () => {
    const t = makeTest();
    await entitledUser(t, "u-send-id");
    const cancel = (clientSendId: string) =>
      asUser(t, "u-send-id").mutation(api.sends.cancel, { clientSendId });
    await expect(cancel("short")).rejects.toThrow(/invalid_argument/);
    await expect(cancel("x".repeat(65))).rejects.toThrow(/invalid_argument/);
    await expect(cancel("has space in it")).rejects.toThrow(/invalid_argument/);
    expect(await cancel("0192f3a0-0000-7000-8000-000000000001")).toEqual({ result: "cancelled" });
  });

  it("rate-limits tombstone creation per user", async () => {
    const t = makeTest();
    await entitledUser(t, "u-send-rate");
    const cancel = (n: number) =>
      asUser(t, "u-send-rate").mutation(api.sends.cancel, {
        clientSendId: `send-${String(n).padStart(6, "0")}`,
      });
    for (let i = 0; i < 60; i++) await cancel(i);
    await expect(cancel(999)).rejects.toThrow(/rate_limited/);
    // Cancelling one that exists creates nothing and is not limited.
    expect(await cancel(0)).toEqual({ result: "cancelled" });
  });
});
