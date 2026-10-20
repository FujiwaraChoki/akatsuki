import { describe, expect, it } from "vitest";
import {
  addAttachment,
  canUndo,
  commitPending,
  composeFromThread,
  draftContent,
  draftSignature,
  emptyCompose,
  isBlank,
  MAX_ATTACHMENT_BYTES,
  type MessageMetaLike,
  parseAddress,
  parseAddressList,
  prefixedSubject,
  removeAttachment,
  type SendPhase,
  sendReducer,
  sendStatus,
  updateAttachment,
  validateCompose,
} from "../src/vm/compose.ts";

const ME = new Set(["me@example.com"]);
const meta = (over: Partial<MessageMetaLike>): MessageMetaLike => ({
  _id: "m1",
  accountId: "acc",
  from: { name: "Ana", email: "ana@x.io" },
  to: [{ email: "me@example.com" }],
  cc: [],
  replyTo: [],
  subject: "Plan",
  date: 1,
  isOutbound: false,
  isDraft: false,
  ...over,
});

describe("addresses", () => {
  it("parses names, quotes and bare addresses", () => {
    expect(parseAddress("Ana Lopez <Ana@X.io>")).toEqual({ name: "Ana Lopez", email: "ana@x.io" });
    expect(parseAddress('"Lopez, Ana" <ana@x.io>')).toEqual({ name: "Lopez, Ana", email: "ana@x.io" });
    expect(parseAddress("mailto:bob@y.org")).toEqual({ email: "bob@y.org" });
    expect(parseAddress("bob")).toBeNull();
    expect(parseAddressList('"Lopez, Ana" <ana@x.io>, bob@y.org; nope\nana@x.io')).toEqual({
      valid: [{ name: "Lopez, Ana", email: "ana@x.io" }, { email: "bob@y.org" }],
      invalid: ["nope"],
    });
  });

  it("does not stack subject prefixes", () => {
    expect(prefixedSubject("reply", "Plan")).toBe("Re: Plan");
    expect(prefixedSubject("reply", "RE: Plan")).toBe("RE: Plan");
    expect(prefixedSubject("forward", "Fw: Plan")).toBe("Fw: Plan");
    expect(prefixedSubject("forward", "Re: Plan")).toBe("Fwd: Re: Plan");
  });
});

describe("replies", () => {
  const messages = [
    meta({
      _id: "m1",
      date: 1,
      to: [{ email: "me@example.com" }, { email: "bo@x.io" }],
      cc: [{ email: "cy@x.io" }],
    }),
    meta({
      _id: "m2",
      date: 2,
      from: { email: "me@example.com" },
      to: [{ email: "ana@x.io" }],
      isOutbound: true,
    }),
    meta({ _id: "m3", date: 3, isDraft: true }),
  ];
  const base = {
    clientDraftId: "d1",
    threadId: "t1",
    subject: "Plan",
    selfEmails: ME,
    identityEmail: "me@example.com",
  };

  it("replies to the newest message from someone else", () => {
    const s = composeFromThread("reply", { ...base, messages });
    expect(s.inReplyToMessageId).toBe("m1");
    expect(s.to).toEqual([{ name: "Ana", email: "ana@x.io" }]);
    expect(s.cc).toEqual([]);
    expect(s.subject).toBe("Re: Plan");
    expect(s.accountId).toBe("acc");
  });

  it("reply all copies everyone except the user", () => {
    const s = composeFromThread("replyAll", { ...base, messages });
    expect(s.cc.map((a) => a.email)).toEqual(["bo@x.io", "cy@x.io"]);
    expect(s.showCcBcc).toBe(true);
  });

  it("honours Reply-To and own threads", () => {
    const rt = composeFromThread("reply", {
      ...base,
      messages: [meta({ replyTo: [{ email: "list@x.io" }] })],
    });
    expect(rt.to).toEqual([{ email: "list@x.io" }]);
    const own = composeFromThread("reply", {
      ...base,
      messages: [meta({ from: { email: "me@example.com" }, to: [{ email: "zed@x.io" }], isOutbound: true })],
    });
    expect(own.to).toEqual([{ email: "zed@x.io" }]);
  });

  it("forwards with Fwd: and the source message", () => {
    const s = composeFromThread("forward", { ...base, messages });
    expect(s.to).toEqual([]);
    expect(s.forwardOfMessageId).toBe("m1");
    expect(s.subject).toBe("Fwd: Plan");
  });
});

describe("drafts", () => {
  it("validates and builds DraftContent with pending recipients committed", () => {
    let s = emptyCompose("d1", "acc", "me@example.com");
    expect(isBlank(s)).toBe(true);
    expect(validateCompose(s).errors).toEqual(["Add at least one recipient."]);
    s = { ...s, pending: { to: "ana@x.io, oops", cc: "", bcc: "" }, text: "Hi" };
    expect(validateCompose(s).errors).toEqual(['"oops" is not an email address.']);
    expect(validateCompose(s).warnings).toEqual(["The subject is empty."]);
    s = commitPending(s, "to");
    expect(s.to).toEqual([{ email: "ana@x.io" }]);
    expect(s.pending.to).toBe("oops");
    s = { ...s, pending: { to: "", cc: "bo@x.io", bcc: "" }, subject: "Hi" };
    const d = draftContent(s);
    expect(d).toEqual({
      accountId: "acc",
      identityEmail: "me@example.com",
      to: [{ email: "ana@x.io" }],
      cc: [{ email: "bo@x.io" }],
      bcc: [],
      subject: "Hi",
      html: null,
      text: "Hi",
      attachments: [],
    });
    expect(validateCompose(s)).toEqual({ errors: [], warnings: [] });
    expect(draftSignature(s)).not.toBe(draftSignature({ ...s, text: "Hi!" }));
  });

  it("attachments join the draft once uploaded and block the send until then", () => {
    let s = { ...emptyCompose("d1", "acc", "me@example.com"), to: [{ email: "a@b.co" }], subject: "Plan" };
    const sig = draftSignature(s);
    s = addAttachment(s, {
      key: "k1",
      filename: "plan.pdf",
      size: 2048,
      mimeType: "application/pdf",
      status: "uploading",
    });
    expect(isBlank({ ...emptyCompose("d2", "acc", "me@example.com"), attachments: s.attachments })).toBe(
      false,
    );
    expect(validateCompose(s).errors).toEqual(["Wait for the attachments to finish uploading."]);
    expect(draftContent(s).attachments).toEqual([]);
    expect(draftSignature(s)).toBe(sig);
    s = updateAttachment(s, "k1", { status: "ready", uploadId: "u1" });
    expect(validateCompose(s)).toEqual({ errors: [], warnings: [] });
    expect(draftContent(s).attachments).toEqual([{ uploadId: "u1" }]);
    expect(draftSignature(s)).not.toBe(sig);
    s = addAttachment(s, {
      key: "k2",
      filename: "film.mov",
      size: MAX_ATTACHMENT_BYTES,
      mimeType: "video/quicktime",
      status: "ready",
      uploadId: "u2",
    });
    expect(validateCompose(s).errors).toEqual(["Attachments are limited to 25 MB per message."]);
    s = updateAttachment(s, "k2", { status: "error", error: "Over 25 MB." });
    expect(validateCompose(s).errors[0]).toBe("Remove the attachments that failed to upload.");
    s = removeAttachment(s, "k2");
    expect(validateCompose(s).errors).toEqual([]);
    // A local copy saved before attachments existed still works.
    const { attachments: _drop, ...old } = s;
    expect(draftContent(old).attachments).toEqual([]);
  });

  it("requires a sender identity", () => {
    const s = { ...emptyCompose("d1", null, null), to: [{ email: "a@b.co" }] };
    expect(validateCompose(s).errors).toEqual(["Choose the address to send from."]);
  });
});

describe("undo send", () => {
  const describe2 = () => "Tue 08:00";
  it("schedules, then undoes within the window", () => {
    let p: SendPhase = { kind: "idle" };
    p = sendReducer(p, { type: "submit", clientSendId: "s1", later: false });
    expect(canUndo(p, 0)).toBe(true);
    expect(sendStatus(p, 0, describe2)).toBe("Sending…");
    p = sendReducer(p, { type: "accepted", sendAt: 10_000, state: "scheduled" });
    expect(p).toEqual({ kind: "scheduled", clientSendId: "s1", sendAt: 10_000, later: false });
    expect(sendStatus(p, 1_500, describe2)).toBe("Sending in 9 s");
    expect(canUndo(p, 9_999)).toBe(true);
    expect(canUndo(p, 10_000)).toBe(false);
    p = sendReducer(p, { type: "undo" });
    expect(p.kind).toBe("cancelling");
    p = sendReducer(p, { type: "cancel_result", result: "cancelled" });
    expect(p.kind).toBe("cancelled");
  });

  it("undo before the server answered wins over the late acceptance", () => {
    let p: SendPhase = sendReducer({ kind: "idle" }, { type: "submit", clientSendId: "s1", later: false });
    p = sendReducer(p, { type: "undo" });
    p = sendReducer(p, { type: "accepted", sendAt: 5, state: "scheduled" });
    expect(p.kind).toBe("cancelling");
    p = sendReducer(p, { type: "cancel_result", result: "too_late" });
    expect(p.kind).toBe("too_late");
  });

  it("handles immediate sends, tombstones, later sends and errors", () => {
    const s = sendReducer({ kind: "idle" }, { type: "submit", clientSendId: "s1", later: false });
    expect(sendReducer(s, { type: "accepted", sendAt: 0, state: "due" }).kind).toBe("sent");
    expect(sendReducer(s, { type: "accepted", sendAt: 0, state: "cancelled" }).kind).toBe("cancelled");
    const later = sendReducer(
      sendReducer({ kind: "idle" }, { type: "submit", clientSendId: "s2", later: true }),
      { type: "accepted", sendAt: 99, state: "scheduled" },
    );
    expect(sendStatus(later, 0, describe2)).toBe("Scheduled for Tue 08:00");
    expect(canUndo(later, 1_000)).toBe(true);
    expect(sendReducer(later, { type: "elapsed" }).kind).toBe("scheduled");
    expect(sendReducer(s, { type: "error", message: "Could not reach the server." })).toEqual({
      kind: "failed",
      clientSendId: "s1",
      message: "Could not reach the server.",
    });
  });
});
