// Read receipts end to end in Convex (§3.2 readReceipts, §4.2 readReceipts.forMessage, D-239, D-412): the worker
// records a draft that carries the pixel, schedules it, the sent message arrives, the pixel is fetched.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../_generated/api";
import type { Id } from "../_generated/dataModel";
import {
  asUser,
  asWorker,
  connectGmail,
  entitledUser,
  makeTest,
  message,
  setEnv,
  type T,
  WORKER_ID,
} from "./setup.helpers";

const TOKEN = "AbCdEfGhIjKlMnOpQrStUv"; // 22 chars, base64url
const RFC = "0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f@gmail.com";

async function draft(t: T, accountId: Id<"mailAccounts">, baseVersion: number, readReceiptToken?: string) {
  const r = await asWorker(t).mutation(api.worker.recordDraft, {
    userId: "rr",
    accountId,
    clientDraftId: "draft-rr",
    baseVersion,
    providerDraftId: "r-rr",
    providerMessageId: "pm-rr",
    rfcMessageId: `<${RFC}>`,
    subject: "Proposal",
    to: [{ email: "bob@example.com" }],
    cc: [],
    bcc: [],
    readReceiptToken,
  });
  if ("conflict" in r) throw new Error("conflict");
  return r;
}

const receipts = (t: T) => t.run(async (ctx) => ctx.db.query("readReceipts").collect());

describe("read receipts", () => {
  beforeEach(() => setEnv());

  it("scheduleSend with a receipt, the sent message, an open: forMessage reports it", async () => {
    vi.useFakeTimers();
    try {
      const t = makeTest();
      await entitledUser(t, "rr");
      const { accountId } = await connectGmail(t, "rr");
      const d1 = await draft(t, accountId, 0, TOKEN);
      // Autosaves keep one row (the token is stable for the draft's Message-ID).
      const d2 = await draft(t, accountId, d1.version, TOKEN);
      expect(await receipts(t)).toHaveLength(1);
      expect((await receipts(t))[0]).toMatchObject({
        userId: "rr",
        token: TOKEN,
        draftId: d2.draftId,
        opens: [],
      });

      const s = await asWorker(t).mutation(api.worker.scheduleSend, {
        userId: "rr",
        accountId,
        clientSendId: "send-rr",
        draftId: d2.draftId,
        kind: "undo",
        sendAt: Date.now() + 10_000,
        preview: { subject: "Proposal", to: [{ email: "bob@example.com" }] },
      });
      expect(s.state).toBe("scheduled");

      // The sent copy reaches Convex through sync.
      await asWorker(t).mutation(api.worker.upsertThreads, {
        accountId,
        workerId: WORKER_ID,
        threads: [
          {
            providerThreadId: "thr-rr",
            complete: true,
            messages: [
              message({
                providerMessageId: "sent-rr",
                rfcMessageId: `<${RFC}>`,
                from: { email: "rr@gmail.com" },
                to: [{ email: "bob@example.com" }],
                labelKeys: ["sys:sent"],
                isUnread: false,
                isOutbound: true,
              }),
            ] as never,
          },
        ],
      });
      const messageId = await t.run(async (ctx) => {
        const m = await ctx.db
          .query("messages")
          .withIndex("by_account_provider", (q) =>
            q.eq("accountId", accountId).eq("providerMessageId", "sent-rr"),
          )
          .unique();
        return m!._id;
      });
      const rr = asUser(t, "rr");
      expect(await rr.query(api.readReceipts.forMessage, { messageId })).toEqual({ opens: [] });

      await asWorker(t).mutation(api.worker.recordReadReceipt, { token: TOKEN, at: 1_000, proxied: true });
      await asWorker(t).mutation(api.worker.recordReadReceipt, { token: TOKEN, at: 2_000, proxied: false });
      // Unknown and malformed tokens change nothing.
      await asWorker(t).mutation(api.worker.recordReadReceipt, {
        token: "x".repeat(22),
        at: 3,
        proxied: false,
      });
      await asWorker(t).mutation(api.worker.recordReadReceipt, { token: "short", at: 3, proxied: false });
      expect(await rr.query(api.readReceipts.forMessage, { messageId })).toEqual({
        opens: [
          { at: 1_000, proxied: true },
          { at: 2_000, proxied: false },
        ],
      });
      // Another user cannot read it.
      await entitledUser(t, "other");
      await expect(asUser(t, "other").query(api.readReceipts.forMessage, { messageId })).rejects.toThrow();
    } finally {
      vi.useRealTimers();
    }
  });

  it("a draft saved again without the pixel drops its unopened row; bad tokens are refused", async () => {
    const t = makeTest();
    await entitledUser(t, "rr");
    const { accountId } = await connectGmail(t, "rr");
    const d1 = await draft(t, accountId, 0, TOKEN);
    expect(await receipts(t)).toHaveLength(1);
    await draft(t, accountId, d1.version);
    expect(await receipts(t)).toHaveLength(0);
    await expect(draft(t, accountId, 2, "not a token")).rejects.toThrow();
  });

  it("a message without a receipt has none", async () => {
    const t = makeTest();
    await entitledUser(t, "rr");
    const { accountId } = await connectGmail(t, "rr");
    await asWorker(t).mutation(api.worker.upsertThreads, {
      accountId,
      workerId: WORKER_ID,
      threads: [
        { providerThreadId: "thr-plain", complete: true, messages: [message({ isOutbound: true })] as never },
      ],
    });
    const messageId = await t.run(async (ctx) => (await ctx.db.query("messages").first())!._id);
    expect(await asUser(t, "rr").query(api.readReceipts.forMessage, { messageId })).toBeNull();
  });
});
