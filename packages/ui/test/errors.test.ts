import { actionsFor } from "@akatsuki/mail-model";
import { ALL_ERROR_CODES, WireError } from "@akatsuki/wire/errors";
import { describe, expect, it } from "vitest";
import {
  actionVerb,
  describeOpFailure,
  ERROR_COPY,
  errorCopy,
  errorMessage,
  maskAccount,
} from "../src/errors.ts";

describe("ERROR_COPY", () => {
  it("has copy for every §15 code, and nothing else", () => {
    expect(Object.keys(ERROR_COPY).sort()).toEqual([...ALL_ERROR_CODES].sort());
  });

  it("is plain sentences with lowercase reasons, no emoji, never 'open source'", () => {
    for (const [code, copy] of Object.entries(ERROR_COPY)) {
      expect(copy.title, code).toMatch(/^[A-Z].*\.$/);
      // Lowercase so it reads after a colon; acronyms (AI, IMAP) keep their case.
      expect(copy.reason, code).toMatch(/^([a-z]|AI\b)/);
      expect(copy.reason, code).not.toMatch(/\.$/);
      expect(`${copy.title} ${copy.reason}`, code).not.toMatch(/\p{Extended_Pictographic}/u);
      expect(`${copy.title} ${copy.reason}`.toLowerCase(), code).not.toContain("open source");
    }
  });

  it("falls back to internal for unknown codes", () => {
    expect(errorCopy("from_the_future")).toBe(ERROR_COPY.internal);
    expect(errorCopy("not_implemented").title).toBe("This is not available yet.");
    expect(errorCopy("key_missing").remedy).toBe("settings");
  });
});

describe("describeOpFailure", () => {
  it("builds the §5.7 sentence", () => {
    expect(
      describeOpFailure({
        verb: actionVerb(actionsFor({ provider: "gmail" }).archive()),
        subject: "Subject",
        account: "sami@samihindi.com",
        code: "not_found",
      }),
    ).toBe("Couldn't archive 'Subject' in sami@…: the message no longer exists.");
  });

  it("clips long subjects and handles missing ones", () => {
    const s = describeOpFailure({ verb: "star", subject: "x".repeat(100), code: "provider_transient" });
    expect(s).toBe(`Couldn't star '${"x".repeat(59)}…': the mail provider had a temporary problem.`);
    expect(describeOpFailure({ verb: "send", subject: "  ", code: "provider_quota" })).toBe(
      "Couldn't send '(no subject)': the mailbox is full or a sending limit was reached.",
    );
  });

  it("names every helper's action", () => {
    const gmail = actionsFor({ provider: "gmail" });
    const graph = actionsFor({ provider: "graph" }, [{ _id: "archive01", exclusive: true, role: "archive" }]);
    expect(actionVerb(gmail.archive())).toBe("archive");
    expect(actionVerb(graph.archive())).toBe("move");
    expect(actionVerb(gmail.spam())).toBe("report");
    expect(actionVerb(graph.spam())).toBe("report");
    expect(actionVerb(gmail.snooze())).toBe("snooze");
    expect(actionVerb(graph.snooze())).toBe("snooze");
    expect(actionVerb(gmail.moveToInbox())).toBe("move to Inbox");
    expect(actionVerb(graph.moveToInbox())).toBe("move to Inbox");
    expect(actionVerb(gmail.markRead())).toBe("mark as read");
    expect(actionVerb(gmail.markUnread())).toBe("mark as unread");
    expect(actionVerb(gmail.star())).toBe("star");
    expect(actionVerb(gmail.unstar())).toBe("unstar");
    expect(actionVerb(gmail.addLabel("abcdefgh"))).toBe("label");
    expect(actionVerb(gmail.removeLabel("abcdefgh"))).toBe("remove the label from");
    expect(actionVerb(gmail.trash())).toBe("move to Trash");
    expect(actionVerb(gmail.untrash())).toBe("restore");
    expect(actionVerb(graph.deleteForever())).toBe("delete");
    expect(actionVerb(gmail.unsubscribe("mailto"))).toBe("unsubscribe from");
  });
});

describe("helpers", () => {
  it("masks accounts and reads thrown errors", () => {
    expect(maskAccount("sami@samihindi.com")).toBe("sami@…");
    expect(maskAccount("weird")).toBe("weird");
    expect(errorMessage(WireError.of("reauth_required"))).toBe("Reconnect this account to keep it in sync.");
    expect(errorMessage({ data: { code: "rate_limited", message: "x", retryable: true } })).toBe(
      "Too many requests. Try again in a moment.",
    );
    expect(errorMessage(new Error("boom"))).toBe("Something went wrong. Try again.");
  });
});
