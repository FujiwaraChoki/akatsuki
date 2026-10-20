// worker.threadIdsByProvider (DECISIONS D-236): maps provider search hits to Convex thread ids.
import { beforeEach, describe, expect, it } from "vitest";
import { api } from "../_generated/api";
import {
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  seedThread,
  setEnv,
} from "./setup.helpers";

describe("worker.threadIdsByProvider", () => {
  beforeEach(() => setEnv());

  it("returns ids for known provider threads of the account only", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    const threadId = await seedThread(t, accountId, "t-1", [message()]);
    const rows = await asWorker(t).query(api.worker.threadIdsByProvider, {
      accountId,
      providerThreadIds: ["t-1", "t-unknown"],
    });
    expect(rows).toEqual([{ providerThreadId: "t-1", threadId }]);
  });

  it("is a worker-only function and caps the batch", async () => {
    const t = makeTest();
    await entitledUser(t, "alice");
    const { accountId } = await connectGmail(t, "alice");
    await expect(
      asUser(t, "alice").query(api.worker.threadIdsByProvider, { accountId, providerThreadIds: [] }),
    ).rejects.toThrow();
    await expect(
      asWorker(t).query(api.worker.threadIdsByProvider, {
        accountId,
        providerThreadIds: Array.from({ length: 101 }, (_, i) => `t-${i}`),
      }),
    ).rejects.toThrow();
  });
});
