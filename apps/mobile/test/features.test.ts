// View-models of the features added in D-460–D-469: multi-select and bulk actions, write with AI and rewrite,
// dictation, IMAP autodetect, labels management, scheduled messages, per-feature AI routes, storage failure copy.
import { type AiProfile, type AiSettings, EMPTY_AI_SETTINGS } from "@akatsuki/ai";
import { describe, expect, it } from "vitest";
import {
  canTranscribe,
  draftFromProfile,
  featureRouteSummary,
  newDraft,
  profileFromDraft,
  routeChoices,
  setFeatureRoute,
} from "../src/vm/aiSettings.ts";
import { bodyWithAi, instructionProblem, rewriteEdit, rewriteRange, writeEdit } from "../src/vm/composeAi.ts";
import {
  audioFilename,
  audioMimeType,
  type DictationPhase,
  dictationReducer,
  elapsedLabel,
  insertTranscript,
  languageHint,
  multipartBody,
  transcriptionsUrl,
} from "../src/vm/dictation.ts";
import {
  applyDiscovery,
  discoveryNote,
  domainToDetect,
  EMPTY_IMAP_FORM,
  editImapField,
  imapConnectBody,
  imapProblems,
  toggleTls,
} from "../src/vm/imapForm.ts";
import { MOBILE_SHORTCUTS, MOBILE_UNSUPPORTED } from "../src/vm/keyboard.ts";
import {
  deleteLabelMessage,
  editableLabels,
  labelNameProblem,
  labelsByAccount,
  renamableFolders,
} from "../src/vm/labels.ts";
import { scheduledRows } from "../src/vm/scheduled.ts";
import {
  addSelected,
  bulkActionForShortcut,
  bulkActions,
  EMPTY_SELECTION,
  extendSelection,
  isSelecting,
  pruneSelection,
  selectAll,
  selectionTitle,
  toggleSelected,
} from "../src/vm/selection.ts";
import { storageFailure } from "../src/vm/storage.ts";
import { aiFeatureStatus, THREAD_AI_COPY } from "../src/vm/threadAi.ts";

const order = ["a", "b", "c", "d", "e"];

describe("multi-select", () => {
  it("toggles rows and keeps the first one as the anchor", () => {
    let s = toggleSelected(EMPTY_SELECTION, "b");
    expect(isSelecting(s)).toBe(true);
    s = toggleSelected(s, "d");
    expect([...s.ids]).toEqual(["b", "d"]);
    expect(s.anchor).toBe("b");
    s = toggleSelected(s, "b");
    expect(s.anchor).toBe("d");
    expect(toggleSelected(s, "d")).toBe(EMPTY_SELECTION);
    expect(addSelected(s, "d")).toBe(s);
  });
  it("extends from the anchor with Shift+J/K", () => {
    const start = toggleSelected(EMPTY_SELECTION, "b");
    let r = extendSelection(start, order, 1, 1);
    expect(r.cursor).toBe(2);
    expect([...r.selection.ids].sort()).toEqual(["b", "c"]);
    r = extendSelection(r.selection, order, r.cursor, 1);
    expect([...r.selection.ids].sort()).toEqual(["b", "c", "d"]);
    // Upwards past the anchor selects the other side too.
    r = extendSelection(toggleSelected(EMPTY_SELECTION, "c"), order, 2, -1);
    expect([...r.selection.ids].sort()).toEqual(["b", "c"]);
    expect(extendSelection(EMPTY_SELECTION, [], -1, 1)).toEqual({ selection: EMPTY_SELECTION, cursor: -1 });
    // No cursor yet: starts on the first row.
    expect(extendSelection(EMPTY_SELECTION, order, -1, 1).cursor).toBe(0);
  });
  it("selects all, toggles back to none, and prunes rows that left", () => {
    const all = selectAll(EMPTY_SELECTION, order);
    expect(all.ids.size).toBe(5);
    expect(selectionTitle(all, 5)).toBe("All 5 selected");
    expect(selectAll(all, order)).toBe(EMPTY_SELECTION);
    const pruned = pruneSelection(all, ["a", "c"]);
    expect([...pruned.ids]).toEqual(["a", "c"]);
    expect(pruneSelection(pruned, ["a", "c", "x"])).toBe(pruned);
    expect(pruneSelection(pruned, ["x"])).toBe(EMPTY_SELECTION);
    expect(selectionTitle(pruned, 5)).toBe("2 selected");
  });
  it("offers the actions that change something for the selection", () => {
    const inbox = { labelKeys: ["sys:inbox", "sys:unread"] };
    const archived = { labelKeys: [] as string[], canMove: true };
    const ids = (rows: Parameters<typeof bulkActions>[0]) => bulkActions(rows).map((a) => a.id);
    expect(ids([inbox, archived])).toEqual([
      "archive",
      "snooze",
      "markRead",
      "trash",
      "star",
      "label",
      "move",
      "spam",
    ]);
    expect(ids([archived])).toEqual([
      "moveToInbox",
      "snooze",
      "markUnread",
      "trash",
      "star",
      "label",
      "move",
      "spam",
    ]);
    const trashed = { labelKeys: ["sys:trash"] };
    expect(ids([trashed])).toEqual(["untrash", "markUnread", "star", "label"]);
    expect(ids([{ labelKeys: ["sys:spam", "sys:starred"] }])).toEqual([
      "notSpam",
      "markUnread",
      "trash",
      "unstar",
      "label",
    ]);
    expect(bulkActions([])).toEqual([]);
    expect(bulkActionForShortcut(bulkActions([inbox]), "thread.toggleRead")?.id).toBe("markRead");
    expect(bulkActionForShortcut(bulkActions([inbox]), "thread.move")).toBeNull();
  });
});

describe("write with AI and rewrite", () => {
  it("streams into the edited range and keeps the rest", () => {
    const body = "Hi Mara,\nthx for teh note.\nBest";
    const range = rewriteRange(body, { start: 9, end: 26 });
    expect(range).toEqual({ start: 9, end: 26, text: "thx for teh note." });
    const edit = rewriteEdit(body, range!);
    expect(bodyWithAi(edit, "Thanks for the")).toBe("Hi Mara,\nThanks for the\nBest");
    expect(bodyWithAi(edit, "Thanks for the note.")).toBe("Hi Mara,\nThanks for the note.\nBest");
    expect(rewriteRange(body, null)).toEqual({ start: 0, end: body.length, text: body });
    expect(rewriteRange("   ", null)).toBeNull();
    expect(rewriteRange(body, { start: 3, end: 3 })?.start).toBe(0);
    expect(bodyWithAi(writeEdit("old draft"), "New text")).toBe("New text");
  });
  it("requires an instruction", () => {
    expect(instructionProblem("  ")).toMatch(/Say what to write/);
    expect(instructionProblem("x".repeat(2001))).toMatch(/under 2,000/);
    expect(instructionProblem("Decline politely")).toBeNull();
  });
});

describe("dictation", () => {
  it("only records after the sheet is open and Record is tapped", () => {
    let p: DictationPhase = { kind: "idle" };
    expect(dictationReducer(p, { type: "start" })).toBe(p);
    p = dictationReducer(p, { type: "open" });
    expect(p.kind).toBe("ready");
    p = dictationReducer(p, { type: "start" });
    p = dictationReducer(p, { type: "recording", at: 1000 });
    expect(p).toEqual({ kind: "recording", startedAt: 1000 });
    p = dictationReducer(p, { type: "stop" });
    expect(p.kind).toBe("transcribing");
    expect(dictationReducer(p, { type: "text" })).toEqual({ kind: "idle" });
    const failed = dictationReducer(p, { type: "fail", message: "No" });
    expect(dictationReducer(failed, { type: "start" }).kind).toBe("starting");
    expect(elapsedLabel(62_400)).toBe("1:02");
  });
  it("inserts the transcript at the cursor with spaces", () => {
    expect(insertTranscript("Hello", null, " world ")).toEqual({ text: "Hello world ", cursor: 12 });
    expect(insertTranscript("Hi there", { start: 2, end: 2 }, "Mara")).toEqual({
      text: "Hi Mara there",
      cursor: 7,
    });
    expect(insertTranscript("", null, "Start")).toEqual({ text: "Start ", cursor: 6 });
    expect(insertTranscript("abc", { start: 1, end: 2 }, "")).toEqual({ text: "abc", cursor: 2 });
  });
  it("builds a byte-exact multipart body", () => {
    const { body, contentType } = multipartBody(
      { language: "de", skipped: undefined },
      { name: "audio", filename: "dictation.m4a", mimeType: "audio/mp4", bytes: new Uint8Array([1, 2, 3]) },
      "XyZ",
    );
    expect(contentType).toBe("multipart/form-data; boundary=XyZ");
    const head =
      '--XyZ\r\ncontent-disposition: form-data; name="language"\r\n\r\nde\r\n' +
      '--XyZ\r\ncontent-disposition: form-data; name="audio"; filename="dictation.m4a"\r\ncontent-type: audio/mp4\r\n\r\n';
    const tail = "\r\n--XyZ--\r\n";
    const enc = new TextEncoder();
    expect([...body]).toEqual([...enc.encode(head), 1, 2, 3, ...enc.encode(tail)]);
    expect(() =>
      multipartBody({}, { name: "a", filename: "f", mimeType: "x", bytes: new Uint8Array() }, "bad\r\n"),
    ).toThrow();
    const injected = new TextDecoder().decode(
      multipartBody(
        { model: "w\r\nx" },
        { name: "file", filename: 'a"b', mimeType: "audio/mp4", bytes: new Uint8Array() },
        "B",
      ).body,
    );
    expect(injected).toContain("w  x");
    expect(injected).toContain('filename="a_b"');
  });
  it("names formats and endpoints", () => {
    expect(audioMimeType("file:///cache/rec-1.m4a")).toBe("audio/mp4");
    expect(audioMimeType("blob:x/rec.webm?x=1")).toBe("audio/webm");
    expect(audioFilename("audio/webm")).toBe("dictation.webm");
    expect(audioFilename("audio/mp4")).toBe("dictation.m4a");
    expect(transcriptionsUrl({ kind: "openai" })).toBe("https://api.openai.com/v1/audio/transcriptions");
    expect(transcriptionsUrl({ kind: "openai_compatible", baseUrl: "http://192.168.1.9:8000/v1/" })).toBe(
      "http://192.168.1.9:8000/v1/audio/transcriptions",
    );
    expect(() => transcriptionsUrl({ kind: "openai_compatible" })).toThrow();
    expect(languageHint("de-CH")).toBe("de");
    expect(languageHint(null)).toBeUndefined();
  });
});

const profile = (p: Partial<AiProfile> & Pick<AiProfile, "profileId" | "kind">): AiProfile => ({
  label: p.kind,
  models: {},
  keyOnDevices: [],
  ...p,
});

describe("AI routes and dictation status", () => {
  const anthropic = profile({ profileId: "p-ant", kind: "anthropic", label: "Claude" });
  const whisper = profile({
    profileId: "p-wh",
    kind: "openai_compatible",
    label: "Whisper",
    baseUrl: "http://192.168.1.9:8000/v1",
    models: { transcription: "whisper-1" },
  });
  const settings: AiSettings = {
    ...EMPTY_AI_SETTINGS,
    profiles: [anthropic, whisper],
    routes: [{ feature: "default", profileId: "p-ant", enabled: true }],
    consent: { grantedAt: 1, version: 1, autoTriage: false, syncVoiceProfile: false },
  };
  const env = (s: AiSettings) => ({ settings: s, managedAi: false, apiConfigured: true, hasKey: () => true });

  it("refuses dictation on a provider that cannot transcribe, and routes it elsewhere", () => {
    expect(aiFeatureStatus("dictation", env(settings)).status).toBe("no_dictation");
    expect(THREAD_AI_COPY.no_dictation).toMatch(/cannot transcribe/);
    expect(routeChoices(settings, "dictation").map((p) => p.profileId)).toEqual(["p-wh"]);
    expect(routeChoices(settings, "compose")).toHaveLength(2);
    const routed = { ...settings, routes: setFeatureRoute(settings, "dictation", "p-wh") };
    expect(aiFeatureStatus("dictation", env(routed)).status).toBe("ready");
    expect(featureRouteSummary(routed, "dictation")).toEqual({
      provider: "Whisper",
      egress: "Stays on your local network (192.168.1.9:8000)",
      own: true,
    });
    expect(featureRouteSummary(routed, "compose").provider).toBe("Same as default (Claude)");
    expect(setFeatureRoute(routed, "dictation", null)).toEqual(settings.routes);
    expect(
      aiFeatureStatus("compose", env({ ...settings, consent: { ...settings.consent, grantedAt: undefined } }))
        .status,
    ).toBe("consent");
  });
  it("names the managed dictation route's retention", () => {
    const managed = profile({ profileId: "p-ak", kind: "akatsuki", label: "Akatsuki AI" });
    const s = {
      ...settings,
      profiles: [managed],
      routes: [{ feature: "default" as const, profileId: "p-ak", enabled: true }],
    };
    expect(featureRouteSummary(s, "dictation").egress).toBe(
      "Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring",
    );
    expect(aiFeatureStatus("dictation", env(s)).status).toBe("not_entitled");
    expect(aiFeatureStatus("dictation", { ...env(s), managedAi: true }).status).toBe("ready");
  });
  it("keeps the dictation model in the profile", () => {
    const d = {
      ...newDraft("p-oa", "openai"),
      model: "gpt-5.5",
      transcriptionModel: "gpt-4o-mini-transcribe",
    };
    const p = profileFromDraft(d);
    expect(p.models.transcription).toBe("gpt-4o-mini-transcribe");
    expect(draftFromProfile(p).transcriptionModel).toBe("gpt-4o-mini-transcribe");
    expect(
      profileFromDraft({ ...newDraft("p-a", "anthropic"), transcriptionModel: "x" }).models.transcription,
    ).toBeUndefined();
    expect(canTranscribe("ollama")).toBe(false);
  });
});

describe("IMAP autodetect", () => {
  const found = {
    imap: { host: "imap.fastmail.com", port: 993, tls: "implicit" as const },
    smtp: { host: "smtp.fastmail.com", port: 465, tls: "implicit" as const },
    username: "",
    source: "well_known" as const,
    appPasswordRequired: true,
  };
  it("fills untouched fields and leaves edited ones alone", () => {
    let f = editImapField(EMPTY_IMAP_FORM, "email", "ana@fastmail.com");
    expect(domainToDetect(f)).toBe("fastmail.com");
    f = editImapField(f, "smtpPort", "587");
    expect(f.smtpTls).toBe("starttls");
    f = applyDiscovery(f, found, "fastmail.com");
    expect(f).toMatchObject({
      username: "ana@fastmail.com",
      imapHost: "imap.fastmail.com",
      imapPort: "993",
      smtpHost: "smtp.fastmail.com",
      smtpPort: "587",
      smtpTls: "starttls",
      detectedFor: "fastmail.com",
    });
    expect(domainToDetect(f)).toBeNull();
    expect(discoveryNote(f)).toMatch(/app password/);
    expect(imapProblems(f)).toEqual(["Enter the password."]);
    f = editImapField(f, "password", "secret");
    expect(imapProblems(f)).toEqual([]);
    expect(imapConnectBody({ ...f, imapHost: "IMAP.Fastmail.com " })).toEqual({
      email: "ana@fastmail.com",
      username: "ana@fastmail.com",
      password: "secret",
      imap: { host: "imap.fastmail.com", port: 993, tls: "implicit" },
      smtp: { host: "smtp.fastmail.com", port: 587, tls: "starttls" },
    });
    expect(
      discoveryNote(applyDiscovery(f, { ...found, source: "guess", appPasswordRequired: false }, "x.y")),
    ).toMatch(/guess/);
    const t = toggleTls(EMPTY_IMAP_FORM, "smtp");
    expect(t.smtpTls).toBe("starttls");
    expect(applyDiscovery(t, found, "fastmail.com").smtpTls).toBe("starttls");
    expect(imapProblems(editImapField(f, "imapPort", "99999"))).toContain(
      "The IMAP port is a number from 1 to 65535.",
    );
  });
});

describe("labels", () => {
  const labels = [
    { _id: "1", accountId: "A", name: "Receipts", labelKey: "l:1", exclusive: false },
    { _id: "2", accountId: "A", name: "INBOX", labelKey: "sys:inbox", kind: "system", exclusive: false },
    { _id: "3", accountId: "A", name: "Projects", labelKey: "l:3", exclusive: true },
    { _id: "4", accountId: "A", name: "Archive", labelKey: "l:4", role: "archive", exclusive: false },
    { _id: "5", accountId: "B", name: "Family", labelKey: "l:5", exclusive: false },
    { _id: "6", accountId: "A", name: "Going", labelKey: "l:6", exclusive: false, pendingDelete: true },
    { _id: "7", accountId: "A", name: "Bills", labelKey: "l:7", exclusive: false },
  ];
  it("lists only the labels the user made, per mailbox", () => {
    expect(editableLabels(labels).map((l) => l.name)).toEqual(["Bills", "Family", "Receipts"]);
    const groups = labelsByAccount(labels, [
      { _id: "A", email: "a@x.y" },
      { _id: "B", email: "b@x.y", status: "removing" },
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0]!.labels.map((l) => l._id)).toEqual(["7", "1"]);
    // A user folder (not a role folder) can be renamed but is never offered for deletion.
    expect(groups[0]!.folders.map((l) => l._id)).toEqual(["3"]);
    expect(
      renamableFolders([
        ...labels,
        { _id: "9", accountId: "A", name: "Akatsuki/Snoozed", labelKey: "l:9", exclusive: true },
      ]).map((l) => l._id),
    ).toEqual(["3"]);
  });
  it("checks names", () => {
    expect(labelNameProblem(" ", [])).toBe("Give it a name.");
    expect(labelNameProblem("receipts", labels)).toMatch(/already has/);
    expect(labelNameProblem("Receipts", labels, "1")).toBeNull();
    expect(labelNameProblem("x".repeat(226), [])).toMatch(/225/);
    expect(deleteLabelMessage("Bills").title).toBe("Delete “Bills”?");
  });
});

describe("scheduled messages", () => {
  const base = {
    kind: "later" as const,
    draftId: "d",
    preview: { subject: "Plan", to: [{ email: "a@x.y" }] },
  };
  it("lists later sends first, then problems, with their actions", () => {
    const rows = scheduledRows(
      [
        {
          ...base,
          _id: "3",
          clientSendId: "c3",
          sendAt: 30,
          state: "failed",
          error: { code: "stale_schedule" },
        },
        { ...base, _id: "1", clientSendId: "c1", sendAt: 20, state: "scheduled" },
        { ...base, _id: "2", clientSendId: "c2", sendAt: 10, state: "unknown" },
        { ...base, _id: "4", clientSendId: "c4", sendAt: 5, state: "cancelled" },
        { ...base, _id: "5", clientSendId: "c5", sendAt: 5, state: "scheduled", kind: "undo" },
        { ...base, _id: "6", clientSendId: "c6", sendAt: 5, state: "scheduled", draftId: undefined },
      ],
      (at) => `at ${at}`,
    );
    expect(rows.map((r) => r.id)).toEqual(["1", "2", "3"]);
    expect(rows[0]).toMatchObject({ status: "Sends at 20", to: "a@x.y" });
    expect(rows[0]!.actions.map((a) => a.id)).toEqual(["reschedule", "cancel"]);
    expect(rows[1]!.actions.map((a) => a.id)).toEqual(["was_sent", "not_sent"]);
    expect(rows[2]!.status).toMatch(/more than a day overdue/);
  });
});

describe("secure storage failure", () => {
  it("explains instead of spinning", () => {
    const f = storageFailure(new Error("User interaction is not allowed.\n(-25308)"));
    expect(f.title).toBe("Secure storage is unavailable");
    expect(f.body).toMatch(/sign simulator builds ad hoc/);
    expect(f.detail).toBe("User interaction is not allowed. (-25308)");
    expect(storageFailure(undefined).detail).toBeNull();
    expect(storageFailure("x".repeat(400)).detail!.length).toBe(158);
  });
});

describe("keyboard subset", () => {
  it("now covers multi-select, snippets and write with AI", () => {
    for (const id of [
      "select.toggle",
      "select.extendDown",
      "select.extendUp",
      "select.all",
      "compose.snippet",
      "compose.aiWrite",
    ])
      expect(MOBILE_SHORTCUTS.has(id)).toBe(true);
    for (const id of ["labels.rename", "sends.scheduled", "compose.dictate", "compose.rewrite", "search.ai"])
      expect(MOBILE_SHORTCUTS.has(id)).toBe(true);
    expect(MOBILE_UNSUPPORTED.has("thread.openOriginal")).toBe(true);
  });
});

describe("egress provenance (design spec §3.8)", () => {
  it("shortens the packages/ai sentences for mono provenance lines", async () => {
    const { egressShort, provenance } = await import("../src/vm/egress.ts");
    expect(egressShort("Stays on this device")).toBe("On this device");
    expect(egressShort("Stays on your local network (192.168.1.9:8000)")).toBe("On your network");
    expect(egressShort("Sent to Akatsuki for processing, not stored")).toBe("Akatsuki · Not stored");
    expect(
      egressShort("Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring"),
    ).toBe("OpenAI via Akatsuki · Kept up to 30 days");
    expect(egressShort("Leaves this device: the thread you run this on, sent to api.anthropic.com")).toBe(
      "Leaves this device · api.anthropic.com",
    );
    expect(provenance("Summary", null)).toBe("Summary");
  });
});
