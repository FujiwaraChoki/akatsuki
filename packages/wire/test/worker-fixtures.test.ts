// fixtures/worker/*.json ({type, value}): the TypeScript side of the round trip crates/core::wire::worker runs.
// TypeScript types have no runtime, so each fixture is checked against a key table that the compiler ties to
// the type (`satisfies Record<keyof T, ...>`): every required key is present, no unknown key appears, and the
// value survives JSON unchanged.
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  type ConnectComplete,
  type ConnectCompleteResponse,
  type ConnectStart,
  type DraftContent,
  type DraftPut,
  type DraftSaved,
  type ImapConnect,
  type ImapDiscovery,
  type MessageBody,
  type SendAccepted,
  type SendRequest,
  splitQuoted,
  utf8ByteLength,
  utf8OffsetToUtf16,
  type WorkerWireTypeName,
} from "../src/index.ts";

type Keys<T> = { [K in keyof T]-?: "required" | "optional" | Keys<NonNullable<T[K]>> };

const DRAFT: Keys<DraftContent> = {
  accountId: "required",
  identityEmail: "required",
  to: "required",
  cc: "required",
  bcc: "required",
  subject: "required",
  html: "required",
  text: "required",
  inReplyToMessageId: "optional",
  forwardOfMessageId: "optional",
  attachments: "required",
  readReceipt: "optional",
};

const TABLES = {
  ConnectStart: {
    returnTo: "required",
    loginHint: "optional",
    accountId: "optional",
  } satisfies Keys<ConnectStart>,
  ConnectComplete: { grant: "required" } satisfies Keys<ConnectComplete>,
  ConnectCompleteResponse: {
    accountId: "required",
    reconnected: "required",
  } satisfies Keys<ConnectCompleteResponse>,
  ImapDiscovery: {
    imap: { host: "required", port: "required", tls: "required" },
    smtp: { host: "required", port: "required", tls: "required" },
    username: "required",
    source: "required",
    appPasswordRequired: "required",
  } satisfies Keys<ImapDiscovery>,
  ImapConnect: {
    email: "required",
    username: "required",
    password: "required",
    imap: "required",
    smtp: "required",
  } satisfies Keys<ImapConnect>,
  MessageBody: {
    messageId: "required",
    providerMessageId: "required",
    sanitizerVersion: "required",
    html: "required",
    text: "required",
    quoted: "required",
    attachments: "required",
    remoteImageCount: "required",
    trackersRemoved: "required",
    headers: "required",
  } satisfies Keys<MessageBody>,
  DraftPut: { baseVersion: "required", draft: DRAFT, deviceId: "required" } satisfies Keys<DraftPut>,
  DraftSaved: {
    draftId: "required",
    version: "required",
    providerDraftId: "required",
    rfcMessageId: "required",
  } satisfies Keys<DraftSaved>,
  SendRequest: {
    clientSendId: "required",
    clientDraftId: "required",
    baseVersion: "required",
    draft: DRAFT,
    sendAt: "optional",
    undoSeconds: "required",
    deviceId: "required",
  } satisfies Keys<SendRequest>,
  SendAccepted: { sendId: "required", sendAt: "required", state: "required" } satisfies Keys<SendAccepted>,
} satisfies Record<WorkerWireTypeName, unknown>;

const OPTIONAL_OBJECTS = new Set(["SendRequest.draft"]);

function checkKeys(path: string, value: Record<string, unknown>, table: Record<string, unknown>) {
  for (const key of Object.keys(value)) expect(table, `${path}: unknown key ${key}`).toHaveProperty(key);
  for (const [key, spec] of Object.entries(table)) {
    const present = key in value;
    if (spec === "required") expect(present, `${path}: missing ${key}`).toBe(true);
    if (typeof spec === "object" && spec !== null) {
      if (!present && OPTIONAL_OBJECTS.has(`${path}.${key}`)) continue;
      expect(present, `${path}: missing ${key}`).toBe(true);
      checkKeys(`${path}.${key}`, value[key] as Record<string, unknown>, spec as Record<string, unknown>);
    }
    if (present) expect(value[key], `${path}.${key} must not be undefined`).not.toBeUndefined();
  }
}

const dir = new URL("../fixtures/worker/", import.meta.url);
const files = readdirSync(dir)
  .filter((f) => f.endsWith(".json"))
  .sort()
  .map((f) => {
    const raw = readFileSync(new URL(f, dir), "utf8");
    return [f, raw, JSON.parse(raw) as { type: WorkerWireTypeName; value: Record<string, unknown> }] as const;
  });

describe("worker fixtures", () => {
  it("has the shared fixtures", () => expect(files.length).toBeGreaterThanOrEqual(12));

  it.each(files)("%s", (_f, _raw, fx) => {
    const table = TABLES[fx.type];
    expect(table, `no TypeScript type for ${fx.type}`).toBeDefined();
    checkKeys(fx.type, fx.value, table as Record<string, unknown>);
    expect(JSON.parse(JSON.stringify(fx.value))).toEqual(fx.value);
    // Optional fields are omitted, never null (Convex and serde reject null for optional fields).
    const nullable = new Set(["html", "text", "htmlStart", "textStart", "listUnsubscribe", "auth"]);
    JSON.stringify(fx.value, (k, v) => {
      if (v === null) expect(nullable.has(k), `${k} is null`).toBe(true);
      return v;
    });
  });

  it("quoted offsets are UTF-8 bytes: the non-ASCII body cuts at the quote", () => {
    const fx = files.find(([f]) => f === "message-body.non-ascii.json")![2].value as unknown as MessageBody;
    const html = fx.html!;
    const text = fx.text!;
    // The byte offset differs from the UTF-16 index whenever non-ASCII text precedes the quote.
    expect(fx.quoted.htmlStart).not.toBe(html.indexOf("<div data-akatsuki-quote"));
    expect(html.slice(utf8OffsetToUtf16(html, fx.quoted.htmlStart!))).toMatch(/^<div data-akatsuki-quote>/);
    expect(utf8ByteLength(html.slice(0, html.indexOf("<div data-akatsuki-quote")))).toBe(fx.quoted.htmlStart);
    const { visible, quoted } = splitQuoted(text, fx.quoted.textStart);
    expect(quoted.startsWith("Am Mo., 19. Okt.")).toBe(true);
    expect(visible + quoted).toBe(text);
  });

  it("ASCII bodies have equal byte and UTF-16 offsets", () => {
    const fx = files.find(([f]) => f === "message-body.html.json")![2].value as unknown as MessageBody;
    expect(utf8OffsetToUtf16(fx.html!, fx.quoted.htmlStart!)).toBe(fx.quoted.htmlStart);
  });
});
