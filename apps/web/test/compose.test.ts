// Composer logic without React: text ↔ HTML, addresses, snippet variables, reply recipients, draft content,
// autosave (§10.1: 2 s debounce, CAS conflicts, no empty drafts) and send / undo-send (§10.2–10.3).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MessageMeta, ThreadDetail } from "../src/backend/types.ts";
import {
  draftFor,
  emptyDraft,
  type LocalDraft,
  newDraft,
  replyRecipients,
  sendProblem,
  toDraftContent,
} from "../src/compose/model.ts";
import { DraftSaver } from "../src/compose/saver.ts";
import { resetSendFlow, sendDraft, undoSend } from "../src/compose/sendFlow.ts";
import { composer } from "../src/compose/store.ts";
import {
  builtinVariables,
  expandVariables,
  forwardSubject,
  htmlToText,
  parseAddressList,
  replySubject,
  textToHtml,
  variablesIn,
} from "../src/compose/text.ts";
import { memoryStore } from "../src/lib/kv.ts";
import { parseSnoozeInput } from "../src/mail/snoozeTimes.ts";

const me = new Set(["you@example.com"]);

function msg(over: Partial<MessageMeta>): MessageMeta {
  return {
    _id: "m1",
    _creationTime: 0,
    accountId: "acc1",
    threadId: "t1",
    providerMessageId: "p1",
    references: [],
    from: { name: "Mara Okafor", email: "mara@example.com" },
    to: [{ name: "You", email: "you@example.com" }],
    cc: [],
    bcc: [],
    replyTo: [],
    subject: "Draft agenda",
    snippet: "Here is the agenda",
    date: Date.UTC(2026, 9, 20, 9),
    labelKeys: ["sys:inbox"],
    isUnread: false,
    isDraft: false,
    isOutbound: false,
    hasAttachments: false,
    ...over,
  } as MessageMeta;
}

describe("text", () => {
  it("escapes HTML and links only http(s) URLs", () => {
    const html = textToHtml('Hi <b>Mara</b>\nsee https://example.com/a?b=1.\n\njavascript:alert(1) "x"');
    expect(html).toBe(
      '<p>Hi &lt;b&gt;Mara&lt;/b&gt;<br>see <a href="https://example.com/a?b=1">https://example.com/a?b=1</a>.</p>\n<p>javascript:alert(1) &quot;x&quot;</p>',
    );
  });
  it("turns quoted lines into a blockquote and back", () => {
    const html = textToHtml("Thanks\n\n> earlier\n> text");
    expect(html).toBe("<p>Thanks</p>\n<blockquote><p>earlier<br>text</p></blockquote>");
    expect(htmlToText(html)).toBe("Thanks\n\n> earlier\n> text");
  });
  it("reduces HTML to text without scripts", () => {
    expect(htmlToText("<p>Hi {{first_name}},</p><script>x()</script><ul><li>a</li><li>b</li></ul>")).toBe(
      "Hi {{first_name}},\n\n- a\n- b",
    );
  });
  it("parses address lists, dedupes and reports invalid entries", () => {
    const r = parseAddressList(
      'Mara <MARA@example.com>, "Okafor, Mara" <mara@example.com>; jun@example.net, nope',
    );
    expect(r.valid).toEqual([{ name: "Mara", email: "mara@example.com" }, { email: "jun@example.net" }]);
    expect(r.invalid).toEqual(["nope"]);
  });
  it("fills snippet variables and reports the missing ones", () => {
    expect(variablesIn("Hi {{first_name}}, see you {{day}}. {{first_name}}")).toEqual(["first_name", "day"]);
    const vars = builtinVariables(
      [{ name: "Mara Okafor", email: "mara@example.com" }],
      Date.UTC(2026, 9, 20, 12),
      "UTC",
    );
    expect(vars.first_name).toBe("Mara");
    expect(vars.day).toBe("Tuesday");
    expect(expandVariables("Hi {{first_name}}, about {{project}}", vars)).toEqual({
      text: "Hi Mara, about {{project}}",
      missing: ["project"],
    });
  });
  it("prefixes subjects once", () => {
    expect(replySubject("Agenda")).toBe("Re: Agenda");
    expect(replySubject("RE: Agenda")).toBe("RE: Agenda");
    expect(forwardSubject("Re: Agenda")).toBe("Fwd: Agenda");
    expect(forwardSubject("Fwd: Agenda")).toBe("Fwd: Agenda");
  });
});

describe("reply recipients", () => {
  it("replies to the sender, or the Reply-To", () => {
    expect(replyRecipients(msg({}), "reply", me)).toEqual({
      to: [{ name: "Mara Okafor", email: "mara@example.com" }],
      cc: [],
    });
    expect(replyRecipients(msg({ replyTo: [{ email: "team@example.com" }] }), "reply", me).to).toEqual([
      { email: "team@example.com" },
    ]);
  });
  it("reply all keeps everyone but me", () => {
    const m = msg({
      to: [{ email: "you@example.com" }, { email: "jun@example.net" }],
      cc: [{ email: "lena@example.org" }, { email: "MARA@example.com" }],
    });
    expect(replyRecipients(m, "reply_all", me)).toEqual({
      to: [{ name: "Mara Okafor", email: "mara@example.com" }],
      cc: [{ email: "jun@example.net" }, { email: "lena@example.org" }],
    });
  });
  it("replying to my own last message goes to its recipients", () => {
    const m = msg({
      from: { email: "you@example.com" },
      to: [{ email: "jun@example.net" }],
      isOutbound: true,
    });
    expect(replyRecipients(m, "reply", me).to).toEqual([{ email: "jun@example.net" }]);
  });
});

const detail = (): ThreadDetail =>
  ({
    thread: { _id: "t1", accountId: "acc1", subject: "Draft agenda" },
    account: { _id: "acc1", provider: "gmail", email: "you@example.com", caps: {} },
    messages: [msg({}), msg({ _id: "m2" as never, isDraft: true, subject: "Re: Draft agenda" })],
    truncated: false,
    pendingOps: [],
    timers: [],
  }) as unknown as ThreadDetail;
const ctx = {
  accounts: [{ _id: "acc1", email: "you@example.com", provider: "gmail" }] as never,
  identities: [
    { _id: "i1", accountId: "acc1", email: "you@example.com", isDefault: true },
    { _id: "i2", accountId: "acc1", email: "alias@example.com", isDefault: false },
  ] as never,
  myEmails: new Set(["you@example.com", "alias@example.com"]),
};

describe("drafts", () => {
  it("builds a reply to the newest non-draft message from the addressed identity", () => {
    const d = draftFor(detail(), "reply", ctx);
    expect(d).toMatchObject({
      mode: "reply",
      accountId: "acc1",
      identityEmail: "you@example.com",
      subject: "Re: Draft agenda",
      inReplyToMessageId: "m1",
      quoteOf: "m1",
      quoted: null,
      threadId: "t1",
    });
    expect(d.to).toEqual([{ name: "Mara Okafor", email: "mara@example.com" }]);
  });
  it("builds a forward without recipients", () => {
    const d = draftFor(detail(), "forward", ctx);
    expect(d).toMatchObject({ subject: "Fwd: Draft agenda", forwardOfMessageId: "m1", to: [] });
  });
  it("produces DraftContent with the quote and only uploaded attachments", () => {
    const d: LocalDraft = {
      ...draftFor(detail(), "reply", ctx),
      body: "Sounds good.",
      quoted: "On Tue, Mara wrote:\n> Here is the agenda",
      attachments: [
        {
          key: "a",
          filename: "a.pdf",
          size: 3,
          mimeType: "application/pdf",
          status: "ready",
          uploadId: "u1",
        },
        { key: "b", filename: "b.pdf", size: 3, mimeType: "application/pdf", status: "uploading" },
        {
          key: "c",
          filename: "c.pdf",
          size: 3,
          mimeType: "application/pdf",
          status: "ready",
          fromMessageId: "m1" as never,
          partId: "2",
        },
      ],
      readReceipt: true,
    };
    const c = toDraftContent(d, textToHtml)!;
    expect(c.text).toBe("Sounds good.\n\nOn Tue, Mara wrote:\n> Here is the agenda");
    expect(c.html).toContain("<blockquote>");
    expect(c.attachments).toEqual([{ uploadId: "u1" }, { fromMessageId: "m1", partId: "2" }]);
    expect(c.inReplyToMessageId).toBe("m1");
    expect(c.readReceipt).toBe(true);
    expect(sendProblem(d)).toBe("Wait for the attachments to finish uploading.");
  });
  it("refuses to send without recipients or above 25 MB", () => {
    const d = newDraft(ctx);
    expect(sendProblem(d)).toBe("Add at least one recipient.");
    const big = {
      ...d,
      to: [{ email: "a@example.com" }],
      attachments: [
        {
          key: "x",
          filename: "x",
          size: 26 * 1024 * 1024,
          mimeType: "x",
          status: "ready" as const,
          uploadId: "u",
        },
      ],
    };
    expect(sendProblem(big)).toBe("Attachments are limited to 25 MB per message.");
  });
});

describe("autosave", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  // biome-ignore lint/suspicious/noExplicitAny: test doubles of the worker call
  const setup = (putDraft: (id: string, body: any) => Promise<unknown>) => {
    const local = memoryStore<LocalDraft>();
    const statuses: string[] = [];
    const draft = { ...emptyDraft({ accountId: "acc1" as never, identityEmail: "you@example.com" }) };
    const saver = new DraftSaver(draft, {
      worker: { putDraft: putDraft as never },
      local,
      deviceId: "dev1",
      html: textToHtml,
      onChange: (s) => statuses.push(s.status),
    });
    return { saver, local, statuses, draft };
  };

  it("debounces 2 s, keeps the device copy at once, and advances the version", async () => {
    const put = vi.fn(async (_id: string, body: { baseVersion: number }) => ({
      draftId: "d1",
      version: body.baseVersion + 1,
      providerDraftId: "p",
      rfcMessageId: "r",
    }));
    const { saver, local, draft } = setup(put);
    saver.update({ ...draft, body: "H" });
    saver.update({ ...draft, body: "Hi" });
    await vi.advanceTimersByTimeAsync(300);
    expect((await local.get(draft.clientDraftId))?.body).toBe("Hi");
    expect(put).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2000);
    expect(put).toHaveBeenCalledTimes(1);
    expect(put.mock.calls[0]![1]).toMatchObject({ baseVersion: 0, deviceId: "dev1", draft: { text: "Hi" } });
    expect(saver.current().version).toBe(1);
    saver.update({ ...saver.current(), body: "Hi there" });
    await saver.flush();
    expect(put).toHaveBeenCalledTimes(2);
    expect(put.mock.calls[1]![1]).toMatchObject({ baseVersion: 1 });
  });

  it("never creates a provider draft for an empty message", async () => {
    const put = vi.fn();
    const { saver, draft } = setup(put);
    saver.update({ ...draft });
    await vi.advanceTimersByTimeAsync(3000);
    await saver.flush();
    expect(put).not.toHaveBeenCalled();
  });

  it("stops on draft_version_conflict and saves on top of the other version with keepMine", async () => {
    let calls = 0;
    const put = vi.fn(async (_id: string, body: { baseVersion: number }) => {
      calls++;
      if (calls === 1)
        throw Object.assign(new Error("conflict"), { code: "draft_version_conflict", version: 4 });
      return { draftId: "d1", version: body.baseVersion + 1, providerDraftId: "p", rfcMessageId: "r" };
    });
    const { saver, draft, statuses } = setup(put);
    saver.update({ ...draft, body: "Mine" });
    await vi.advanceTimersByTimeAsync(2100);
    expect(saver.status).toBe("conflict");
    expect(saver.conflictVersion).toBe(4);
    saver.update({ ...saver.current(), body: "Mine, edited" });
    await vi.advanceTimersByTimeAsync(5000);
    expect(put).toHaveBeenCalledTimes(1); // waits for the user's choice
    await saver.keepMine();
    expect(put.mock.calls[1]![1]).toMatchObject({ baseVersion: 4, draft: { text: "Mine, edited" } });
    expect(saver.current().version).toBe(5);
    expect(statuses).toContain("saved");
  });
});

describe("send and undo-send", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    composer.reset();
    resetSendFlow();
  });
  afterEach(() => vi.useRealTimers());

  const deps = (over: Partial<Parameters<typeof sendDraft>[2]> = {}) => {
    const toasts: { id?: string; title: string }[] = [];
    const reopened: LocalDraft[] = [];
    const local = memoryStore<LocalDraft>();
    const d = {
      worker: {
        send: vi.fn(async (b: { sendAt?: number; undoSeconds: number }) => ({
          sendId: "s1" as never,
          sendAt: b.sendAt ?? Date.now() + b.undoSeconds * 1000,
          state: "scheduled" as const,
        })),
      },
      mutations: {
        cancelSend: vi.fn(async () => ({ result: "cancelled" as const })),
        setReminder: vi.fn(async () => ({ timerId: "x" })),
      },
      fetchDraft: vi.fn(async () => ({ version: 3, threadId: "t9" }) as never),
      toast: (o: { id?: string; title: string }) => {
        toasts.push(o);
        return o.id ?? "";
      },
      dismiss: () => {},
      local,
      deviceId: "dev1",
      html: textToHtml,
      reopen: (x: LocalDraft) => reopened.push(x),
      timezone: "UTC",
      followUpPoll: { tries: 2, intervalMs: 10 },
      ...over,
    };
    return { d, toasts, reopened, local };
  };
  const ready = (): LocalDraft => ({
    ...newDraft(ctx),
    to: [{ email: "mara@example.com" }],
    subject: "Agenda",
    body: "Hi",
    version: 2,
  });

  it("shows Sending… with Undo, sends once with the final content, then says Sent.", async () => {
    const { d, toasts, local } = deps();
    const draft = ready();
    await local.put(draft.clientDraftId, draft);
    const p = sendDraft(draft, { undoSeconds: 10 }, d as never);
    expect(toasts[0]).toMatchObject({ title: "Sending…" });
    expect(composer.undoable()?.clientSendId).toBe(p.clientSendId);
    expect(d.worker.send).toHaveBeenCalledWith(
      expect.objectContaining({
        clientDraftId: draft.clientDraftId,
        baseVersion: 2,
        undoSeconds: 10,
        deviceId: "dev1",
      }),
    );
    await vi.advanceTimersByTimeAsync(10_100);
    expect(toasts.at(-1)).toMatchObject({ title: "Sent." });
    expect(composer.undoable()).toBeNull();
    expect(await local.get(draft.clientDraftId)).toBeUndefined();
  });

  it("Undo cancels on Convex and reopens the composer with the newest version", async () => {
    const { d, reopened, toasts } = deps();
    const draft = ready();
    const p = sendDraft(draft, { undoSeconds: 10 }, d as never);
    await vi.advanceTimersByTimeAsync(1000);
    expect(await undoSend(p.clientSendId, d as never)).toBe(true);
    expect(d.mutations.cancelSend).toHaveBeenCalledWith({ clientSendId: p.clientSendId });
    expect(reopened[0]).toMatchObject({ clientDraftId: draft.clientDraftId, body: "Hi", version: 3 });
    expect(toasts.at(-1)?.title).toBe("Sending cancelled.");
    await vi.advanceTimersByTimeAsync(20_000);
    expect(toasts.some((t) => t.title === "Sent.")).toBe(false);
  });

  it("too_late keeps the message sent", async () => {
    const { d, reopened, toasts } = deps({
      mutations: { cancelSend: vi.fn(async () => ({ result: "too_late" as const })), setReminder: vi.fn() },
    } as never);
    const p = sendDraft(ready(), { undoSeconds: 5 }, d as never);
    expect(await undoSend(p.clientSendId, d as never)).toBe(false);
    expect(reopened).toHaveLength(0);
    expect(toasts.at(-1)?.title).toBe("Too late to undo. The message was sent.");
  });

  it("send later passes sendAt and offers Undo for 10 s", async () => {
    const { d, toasts } = deps();
    const at = Date.now() + 86_400_000;
    const p = sendDraft(ready(), { undoSeconds: 10, sendAt: at }, d as never);
    expect(d.worker.send).toHaveBeenCalledWith(expect.objectContaining({ sendAt: at }));
    expect(toasts[0]?.title).toMatch(/^Scheduled for /);
    expect(p.kind).toBe("later");
    await vi.advanceTimersByTimeAsync(10_500);
    expect(composer.undoable()).toBeNull();
  });

  it("a follow-up becomes an if_no_reply reminder on the sent thread", async () => {
    const { d } = deps();
    const draft = { ...ready(), followUp: "3d" as const };
    const p = sendDraft(draft, { undoSeconds: 0 }, d as never);
    await vi.advanceTimersByTimeAsync(100);
    expect(d.mutations.setReminder).toHaveBeenCalledWith({
      threadId: "t9",
      at: p.sendAt + 3 * 86_400_000,
      condition: "if_no_reply",
    });
  });

  it("a failed request reopens nothing by itself and offers the draft", async () => {
    const { d, toasts } = deps({
      worker: {
        send: vi.fn(async () => Promise.reject(Object.assign(new Error("x"), { code: "unreachable" }))),
      },
    } as never);
    sendDraft(ready(), { undoSeconds: 10 }, d as never);
    await vi.advanceTimersByTimeAsync(10);
    const t = toasts.at(-1) as { title: string; action?: { label: string } };
    expect(t.title).toMatch(/^Couldn't send/);
    expect(t.action?.label).toBe("Open draft");
    expect(composer.undoable()).toBeNull();
  });
});

describe("natural-language times", () => {
  // Tuesday 2026-10-20 10:00 UTC.
  const now = Date.UTC(2026, 9, 20, 10, 0);
  const at = (s: string) => {
    const t = parseSnoozeInput(s, now, "UTC");
    return t === null ? null : new Date(t).toISOString().slice(0, 16);
  };
  it.each([
    ["in 2 hours", "2026-10-20T12:00"],
    ["an hour", "2026-10-20T11:00"],
    ["end of day", "2026-10-20T17:00"],
    ["this evening", "2026-10-20T19:00"],
    ["tomorrow morning", "2026-10-21T08:00"],
    ["next week", "2026-10-26T08:00"],
    ["this weekend", "2026-10-24T08:00"],
    ["next month", "2026-11-01T08:00"],
    ["next fri", "2026-10-30T08:00"],
    ["nov 3", "2026-11-03T08:00"],
    ["3rd of december at 2pm", "2026-12-03T14:00"],
    ["oct 1", "2027-10-01T08:00"],
    ["in 2 months", "2026-12-20T10:00"],
  ])("%s", (input, expected) => {
    expect(at(input)).toBe(expected);
  });
});
