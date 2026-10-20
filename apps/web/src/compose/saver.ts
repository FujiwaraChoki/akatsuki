// Compose autosave (ARCHITECTURE §10.1): every change goes to the device's compose store at once (IndexedDB,
// a short debounce), and a 2 s debounce writes the provider draft with `PUT /v1/drafts/{clientDraftId}`
// `{baseVersion, draft, deviceId}`. The worker's compare-and-set answers a stale `baseVersion` with
// `409 draft_version_conflict` carrying the current version; the composer then offers to reload the other
// device's copy (`adopt`) or to keep this one (save again on top of that version, D-281).
//
// Only one save is in flight per draft; edits made during a save trigger the next one. Empty drafts (no
// recipient, subject, text or attachment) never create a provider draft.
import type { DraftSaved } from "@akatsuki/wire";
import type { WorkerApi } from "../backend/types.ts";
import type { KvStore } from "../lib/kv.ts";
import { errorCodeOf } from "../lib/outbox.ts";
import { hasContent, type LocalDraft, toDraftContent } from "./model.ts";

export type SaveStatus = "idle" | "pending" | "saving" | "saved" | "error" | "conflict";

export interface SaverDeps {
  worker: Pick<WorkerApi, "putDraft">;
  local: KvStore<LocalDraft>;
  deviceId: string;
  html: (text: string) => string;
  /** Called after every status or version change. */
  onChange?: (s: { status: SaveStatus; draft: LocalDraft; conflictVersion: number | null }) => void;
  debounceMs?: number;
  localDebounceMs?: number;
  now?: () => number;
}

export const AUTOSAVE_MS = 2000;

export class DraftSaver {
  private draft: LocalDraft;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private localTimer: ReturnType<typeof setTimeout> | null = null;
  private inflight: Promise<void> | null = null;
  private edits = 0;
  private savedEdits = 0;
  private disposed = false;
  status: SaveStatus = "idle";
  conflictVersion: number | null = null;
  lastError: unknown = null;

  constructor(
    draft: LocalDraft,
    private deps: SaverDeps,
  ) {
    this.draft = draft;
  }

  current(): LocalDraft {
    return this.draft;
  }

  private now() {
    return this.deps.now?.() ?? Date.now();
  }

  private set(status: SaveStatus) {
    this.status = status;
    this.deps.onChange?.({ status, draft: this.draft, conflictVersion: this.conflictVersion });
  }

  /** Records an edit: local copy soon, provider draft after the debounce. */
  update(next: LocalDraft): void {
    if (this.disposed) return;
    this.draft = { ...next, updatedAt: this.now() };
    this.edits++;
    if (this.localTimer) clearTimeout(this.localTimer);
    this.localTimer = setTimeout(() => void this.writeLocal(), this.deps.localDebounceMs ?? 250);
    if (this.status === "conflict") return; // wait for the user's choice
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.save(), this.deps.debounceMs ?? AUTOSAVE_MS);
    if (this.status !== "saving") this.set("pending");
  }

  async writeLocal(): Promise<void> {
    if (this.localTimer) clearTimeout(this.localTimer);
    this.localTimer = null;
    await this.deps.local.put(this.draft.clientDraftId, this.draft).catch(() => undefined);
  }

  /** Saves now (send, close, blur). Resolves with the latest draft (its `version` is the new baseVersion). */
  async flush(): Promise<LocalDraft> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    await this.writeLocal();
    if (this.inflight) await this.inflight;
    if (this.edits !== this.savedEdits && this.status !== "conflict") await this.save();
    return this.draft;
  }

  private save(): Promise<void> {
    if (this.inflight) return this.inflight;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const p = this.run().finally(() => {
      this.inflight = null;
      // Typed while saving: save again after the debounce.
      if (
        !this.disposed &&
        this.edits !== this.savedEdits &&
        this.status !== "conflict" &&
        this.status !== "error"
      )
        this.timer = setTimeout(() => void this.save(), this.deps.debounceMs ?? AUTOSAVE_MS);
    });
    this.inflight = p;
    return p;
  }

  private async run(): Promise<void> {
    const snapshot = this.draft;
    const editsAtStart = this.edits;
    const content = toDraftContent(snapshot, this.deps.html);
    if (!content || !hasContent(snapshot)) {
      this.savedEdits = editsAtStart;
      this.set(this.draft.version > 0 ? "saved" : "idle");
      return;
    }
    this.set("saving");
    let saved: DraftSaved;
    try {
      saved = await this.deps.worker.putDraft(snapshot.clientDraftId, {
        baseVersion: snapshot.version,
        draft: content,
        deviceId: this.deps.deviceId,
      });
    } catch (e) {
      if (errorCodeOf(e) === "draft_version_conflict") {
        const v =
          (e as { version?: unknown; data?: { version?: unknown } }).version ??
          (e as { data?: { version?: unknown } }).data?.version;
        this.conflictVersion = typeof v === "number" ? v : null;
        this.set("conflict");
        return;
      }
      this.lastError = e;
      this.set("error");
      return;
    }
    this.draft = { ...this.draft, version: saved.version, savedAt: this.now() };
    this.savedEdits = editsAtStart;
    await this.writeLocal();
    this.set("saved");
  }

  /** Waits for a save in flight without starting one (send uses the version it produces). */
  async settle(): Promise<LocalDraft> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    if (this.inflight) await this.inflight.catch(() => undefined);
    return this.draft;
  }

  /** Takes a change the user did not make (the reply quote arriving) without scheduling a save. */
  sync(next: LocalDraft): void {
    this.draft = { ...next, version: this.draft.version, savedAt: this.draft.savedAt };
  }

  /** "Keep mine": save this copy on top of the other device's version. */
  async keepMine(): Promise<LocalDraft> {
    if (this.conflictVersion === null) return this.draft;
    this.draft = { ...this.draft, version: this.conflictVersion };
    this.conflictVersion = null;
    this.status = "pending";
    this.edits++;
    return this.flush();
  }

  /** "Use theirs": replace this copy with the adopted content and version. */
  replace(next: LocalDraft): void {
    this.conflictVersion = null;
    this.draft = next;
    this.savedEdits = this.edits;
    void this.writeLocal();
    this.set("saved");
  }

  /** Retry after an error (offline, worker down). */
  retry(): Promise<void> {
    this.status = "pending";
    return this.save();
  }

  dispose(): void {
    this.disposed = true;
    if (this.timer) clearTimeout(this.timer);
    if (this.localTimer) clearTimeout(this.localTimer);
  }
}
