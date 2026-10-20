// The mock backend: the same `Backend` contract as `../convex.tsx`, over `MockStore`. Used by Vitest, by the
// Playwright smoke tests (`vite build --mode mock`) and for offline demos. Never part of a production build
// (`__AKATSUKI_MOCK__` is false there, so the import in main.tsx is tree-shaken away).
import { useMemo, useState, useSyncExternalStore } from "react";
import { wipeLocalData } from "../../lib/kv.ts";
import type {
  AccountId,
  Backend,
  ListResult,
  ListViewArgs,
  MessageId,
  Mutations,
  SearchArgs,
  SearchResult,
  ThreadId,
} from "../types.ts";
import { mockBody } from "./data.ts";
import { type MockOptions, MockStore } from "./store.ts";

export { DEMO_OTP } from "./data.ts";
export { MockStore } from "./store.ts";

const PAGE = 50;
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function createMockBackend(
  opts: MockOptions & { store?: MockStore } = {},
): Backend & { store: MockStore } {
  const store = opts.store ?? new MockStore(opts);
  const x = store.extras;
  const useVersion = () => useSyncExternalStore(store.subscribe, store.getVersion, store.getVersion);

  function useList(compute: () => import("../types.ts").ThreadRow[], key: string): ListResult {
    const version = useVersion();
    const [limit, setLimit] = useState({ key, n: PAGE });
    const n = limit.key === key ? limit.n : PAGE;
    // biome-ignore lint/correctness/useExhaustiveDependencies: version is the store's change signal
    const all = useMemo(compute, [key, version]);
    return {
      rows: all.slice(0, n),
      status: n >= all.length ? "done" : "more",
      loadMore: (more) => setLimit({ key, n: n + more }),
    };
  }

  const mutations: Mutations = {
    apply: async (a) => store.apply(a.clientOpId, a.threadIds, a.action),
    undo: async (a) => store.undo(a.clientOpId, a.undoClientOpId),
    snooze: async (a) => store.snooze(a.clientOpId, a.threadIds, a.until, a.returnUnread),
    unsnooze: async (a) => store.unsnooze(a.clientOpId, a.threadIds),
    setReminder: async (a) => store.setReminder(a.threadId, a.at, a.condition, a.note),
    bootstrap: async () => null,
    updatePrefs: async (a) => store.updatePrefs(a.patch),
    updateProfile: async (a) => {
      if (a.onboarding) store.setOnboarding(a.onboarding);
    },
    ackFailure: async (a) => store.ackFailure(a.opIds),
    retryOp: async (a) => store.retry(a.opId, a.clientOpId),
    setSenderPref: async (a) => {
      store.setSenderPref(a.sender, a);
      if (a.blocked !== undefined) {
        if (a.blocked) x.blocked.add(a.sender);
        else x.blocked.delete(a.sender);
        store.emit();
      }
    },
    registerDevice: async (a) => {
      if (!x.devices.some((d) => d.deviceId === a.deviceId))
        x.devices = [
          ...x.devices,
          {
            _id: `dev${x.devices.length + 1}` as never,
            _creationTime: store.now(),
            deviceId: a.deviceId,
            kind: a.kind,
            name: a.name,
            appVersion: a.appVersion,
            lastSeenAt: store.now(),
          },
        ];
      store.emit();
      return null;
    },
    cancelReminder: async (a) => {
      store.timers = store.timers.filter((t) => t._id !== a.timerId);
      store.emit();
    },
    discardDraft: async (a) => x.discardDraft(a.clientDraftId),
    cancelSend: async (a) => x.cancelSend(a.clientSendId),
    rescheduleSend: async (a) => x.rescheduleSend(a.clientSendId, a.sendAt),
    resolveUnknownSend: async (a) => x.resolveUnknown(a.clientSendId, a.resolution),
    saveSplit: async (a) => {
      const { splitId, ...fields } = a;
      if (splitId) store.splits = store.splits.map((s) => (s._id === splitId ? { ...s, ...fields } : s));
      else {
        const id = `split${store.splits.length + 1}x` as never;
        store.splits = [
          ...store.splits,
          {
            _id: id,
            _creationTime: store.now(),
            order: store.splits.length,
            labelKey: `split:${id}`,
            ...fields,
          },
        ];
      }
      store.resplit();
      return { splitId: (splitId ?? store.splits[store.splits.length - 1]!._id) as never };
    },
    removeSplit: async (a) => {
      store.splits = store.splits.filter((s) => s._id !== a.splitId);
      store.resplit();
    },
    reorderSplits: async (a) => {
      store.splits = store.splits.map((s) => ({ ...s, order: Math.max(0, a.order.indexOf(s._id)) }));
      store.resplit();
    },
    saveRule: async (a) => x.saveRule(a),
    removeRule: async (a) => {
      x.rules = x.rules.filter((r) => r._id !== a.ruleId);
      store.emit();
    },
    saveSnippet: async (a) => x.saveSnippet(a),
    removeSnippet: async (a) => {
      x.snippets = x.snippets.filter((s) => s._id !== a.snippetId);
      store.emit();
    },
    createLabel: async (a) => store.createLabel(a.accountId, a.name),
    renameLabel: async (a) => store.renameLabel(a.labelId, a.name),
    removeLabel: async (a) => store.removeLabel(a.labelId),
    markSnippetUsed: async (a) => {
      x.snippets = x.snippets.map((s) =>
        s._id === a.snippetId ? { ...s, useCount: s.useCount + 1, lastUsedAt: store.now() } : s,
      );
      store.emit();
    },
    updateIdentity: async (a) => {
      const { identityId, ...patch } = a;
      const target = x.identities.find((i) => i._id === identityId);
      x.identities = x.identities.map((i) =>
        i._id === identityId
          ? { ...i, ...patch }
          : patch.isDefault && target && i.accountId === target.accountId
            ? { ...i, isDefault: false }
            : i,
      );
      store.emit();
    },
    updateAccount: async (a) => {
      const { accountId, ...patch } = a;
      store.accounts = store.accounts.map((acc) => (acc._id === accountId ? { ...acc, ...patch } : acc));
      store.emit();
    },
    setBackfillHorizon: async (a) => {
      store.accounts = store.accounts.map((acc) =>
        acc._id === a.accountId ? { ...acc, backfillHorizonAt: a.horizonAt } : acc,
      );
      store.emit();
    },
    removeAccount: async (a) => {
      store.accounts = store.accounts.map((acc) =>
        acc._id === a.accountId ? { ...acc, status: "removing" as const } : acc,
      );
      store.emit();
    },
    saveAiSettings: async (a) => x.saveAi(a),
    removeDevice: async (a) => {
      x.devices = x.devices.filter((d) => d.deviceId !== a.deviceId);
      store.emit();
    },
    deleteAccount: async () => {
      store.accounts = store.accounts.map((acc) => ({ ...acc, status: "removing" as const }));
      store.emit();
    },
    claimTriage: async () => [],
    setTriage: async (a) => {
      for (const r of a.results) {
        const t = store.threads.get(r.threadId);
        if (t) t.row = { ...t.row, triage: { category: r.category, needsReply: r.needsReply } };
      }
      store.emit();
    },
  };

  const backend: Backend & { store: MockStore } = {
    kind: "mock",
    store,
    Provider: ({ children }) => children,
    loginProviders: ["google", "microsoft"],
    auth: {
      sendMagicLink: async () => {
        await delay(150);
      },
      sendOtp: async () => {
        await delay(150);
      },
      verifyOtp: async (email, otp) => {
        await delay(100);
        store.verifyOtp(email, otp);
      },
      signInSocial: async () => {
        store.signIn("you@example.com");
      },
      signOut: async () => {
        store.signOut();
        await wipeLocalData();
      },
      listSessions: async () => x.sessions,
      revokeSession: async (token) => {
        x.sessions = x.sessions.filter((s) => s.token !== token);
        store.emit();
      },
      deleteUser: async () => {
        store.signOut();
        await wipeLocalData();
      },
    },
    worker: {
      async *threadBodies(threadId) {
        const t = store.thread(threadId);
        if (!t) throw Object.assign(new Error("not_found"), { code: "not_found" });
        await delay(40);
        for (const m of [...t.messages].reverse())
          yield mockBody(m, {
            remoteImages: !t.thread.isBulk ? m === t.messages[t.messages.length - 1] : true,
          });
      },
      async getMessage(messageId: MessageId) {
        for (const t of store.threads.values()) {
          const m = t.messages.find((x) => x._id === messageId);
          if (m) return mockBody(m, { remoteImages: false });
        }
        throw Object.assign(new Error("not_found"), { code: "not_found" });
      },
      async getPartBlob() {
        return new Blob(["%PDF-1.4 mock"], { type: "application/pdf" });
      },
      async createLink() {
        return "data:application/pdf;base64,JVBERi0xLjQgbW9jaw==";
      },
      async connectStart(provider, body) {
        // Like the worker's callback: the account is created when the starter redeems the grant (D-395). The
        // handle names the provider so it survives the full-page navigation back from the "provider".
        const grant = `mock-grant-${provider}`;
        const url = new URL(body.returnTo, window.location.origin);
        url.searchParams.set("status", "pending");
        url.searchParams.set("grant", grant);
        return url.toString();
      },
      async connectComplete(grant) {
        const provider =
          grant === "mock-grant-google" ? "google" : grant === "mock-grant-microsoft" ? "microsoft" : null;
        if (!provider) throw Object.assign(new Error("not_found"), { code: "not_found" });
        const id = store.connect(
          provider === "google" ? "gmail" : "imap",
          provider === "google" ? "you@example.com" : "you@example.net",
        );
        return { accountId: id, reconnected: false };
      },
      async connectImap(body) {
        await delay(200);
        if (body.password === "wrong")
          throw Object.assign(new Error("imap_auth_failed"), { code: "imap_auth_failed" });
        return store.connect("imap", body.email);
      },
      async discoverImap(email) {
        const { discoverImap } = await import("../../onboarding/imapDiscovery.ts");
        // Stands in for the worker's GET /v1/connect/imap/discover (D-396) with one known provider.
        const known = email.trim().toLowerCase().endsWith("@fastmail.com")
          ? async () => ({
              imap: { host: "imap.fastmail.com", port: 993, tls: "implicit" },
              smtp: { host: "smtp.fastmail.com", port: 465, tls: "implicit" },
              username: email.trim(),
              source: "well_known",
              appPasswordRequired: true,
            })
          : null;
        return discoverImap(email, known);
      },
      async search(accountId: AccountId, q: string) {
        return store.search(q, Date.now(), accountId).rows.map((r) => r._id);
      },
      async syncAccount() {},
      async upload(file, filename) {
        await delay(60);
        if (file.size > 25 * 1024 * 1024)
          throw Object.assign(new Error("payload_too_large"), { code: "payload_too_large" });
        return { uploadId: `upl-${filename}-${file.size}`, size: file.size, sha256: "0".repeat(64) };
      },
      async putDraft(clientDraftId, body) {
        await delay(30);
        return x.putDraft(clientDraftId, body);
      },
      async adoptDraft(clientDraftId, messageId) {
        await delay(30);
        return x.adoptDraft(clientDraftId, messageId);
      },
      async deleteDraft(clientDraftId) {
        x.discardDraft(clientDraftId);
      },
      async send(body) {
        await delay(40);
        return x.send(body);
      },
    },
    api: x.api(),
    async fetchThread(threadId) {
      return store.thread(threadId);
    },
    async fetchDraft(clientDraftId) {
      return x.drafts.get(clientDraftId)?.meta ?? null;
    },
    useAuthState() {
      useVersion();
      const me = store.me();
      return me
        ? { status: "signed_in", user: { id: me.userId, email: me.email, name: me.name } }
        : { status: "signed_out" };
    },
    useMe() {
      useVersion();
      return store.me();
    },
    useAccounts() {
      useVersion();
      return store.accounts;
    },
    useLabels() {
      useVersion();
      return store.labels;
    },
    useSplits() {
      useVersion();
      return store.splits;
    },
    useThreadList(view: ListViewArgs | null) {
      const key = view ? JSON.stringify(view) : "none";
      return useList(() => (view ? store.list(view) : []), key);
    },
    useThreadRows(ids: readonly ThreadId[]) {
      useVersion();
      return ids.map((id) => store.threads.get(id)?.row).filter((r) => r !== undefined);
    },
    useThread(threadId) {
      useVersion();
      return threadId ? store.thread(threadId) : null;
    },
    useUnreadCounts(keys, accountId) {
      useVersion();
      return store.unreadCounts(keys, accountId);
    },
    useSearch(args: SearchArgs | null): SearchResult {
      const key = args ? JSON.stringify(args) : "none";
      const version = useVersion();
      // biome-ignore lint/correctness/useExhaustiveDependencies: version is the store's change signal
      const result = useMemo(
        () => (args ? store.search(args.q, args.nowAt, args.accountId) : { rows: [], unsupported: [] }),
        [key, version],
      );
      const list = useList(() => result.rows, key);
      return { ...list, unsupported: result.unsupported };
    },
    useTimers(kind) {
      useVersion();
      return store.timers.filter((t) => t.kind === kind);
    },
    useOpFailures() {
      useVersion();
      return store.failures;
    },
    useSenderPref(sender) {
      useVersion();
      if (!sender) return null;
      return store.senderPrefs.get(sender) ?? null;
    },
    useContacts(q) {
      useVersion();
      if (q === null) return undefined;
      const seen = new Map<string, { email: string; name?: string }>();
      for (const t of store.threads.values())
        for (const p of t.row.participants)
          if (`${p.name ?? ""} ${p.email}`.toLowerCase().includes(q.toLowerCase()) && !seen.has(p.email))
            seen.set(p.email, p.name ? { email: p.email, name: p.name } : { email: p.email });
      return [...seen.values()].slice(0, 6);
    },
    useMutations: () => mutations,
    useConnected() {
      useVersion();
      return store.connected;
    },
    useIdentities(accountId) {
      useVersion();
      return accountId ? x.identities.filter((i) => i.accountId === accountId) : x.identities;
    },
    useSnippets() {
      useVersion();
      return x.snippets;
    },
    useRules() {
      useVersion();
      return x.rules;
    },
    useDevices() {
      useVersion();
      return x.devices;
    },
    useAiSettings() {
      useVersion();
      return x.aiSettings;
    },
    useBilling() {
      useVersion();
      return x.billing;
    },
    useSends(states) {
      useVersion();
      return x.sends.filter((s) =>
        states ? (states as string[]).includes(s.state) : s.state !== "cancelled",
      );
    },
    useDrafts() {
      useVersion();
      return [...x.drafts.values()]
        .map((d) => d.meta)
        .filter((d) => d.state === "editing")
        .sort((a, b) => b.updatedAt - a.updatedAt);
    },
    useReadReceipt(messageId) {
      useVersion();
      return messageId ? null : null;
    },
    useSenderPrefFull(sender) {
      useVersion();
      if (!sender) return null;
      const p = store.senderPrefs.get(sender);
      const blocked = x.blocked.has(sender);
      return p || blocked
        ? { sender, allowRemoteImages: p?.allowRemoteImages ?? false, vip: p?.vip ?? false, blocked }
        : null;
    },
  };
  return backend;
}
