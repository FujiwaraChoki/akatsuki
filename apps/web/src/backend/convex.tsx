// The production backend (ARCHITECTURE §6.2, §16.2): Convex live queries and mutations with optimistic
// updates from @akatsuki/mail-model, Better Auth through @convex-dev/better-auth (crossDomain), and the sync
// worker through @akatsuki/wire's typed client with the user's JWT.
import { type ApplyContext, applyContextFor, SYS, type ThreadAction } from "@akatsuki/mail-model";
import { applyOptimisticUpdate, snoozeOptimisticUpdate } from "@akatsuki/mail-model/optimistic";
import {
  clearAccessToken,
  createApiClient,
  createWorkerClient,
  setAccessTokenSource,
  WireError,
} from "@akatsuki/wire";
import { api } from "@akatsuki/wire/convex";
import { convexClient, crossDomainClient } from "@convex-dev/better-auth/client/plugins";
import { ConvexBetterAuthProvider } from "@convex-dev/better-auth/react";
import { emailOTPClient, magicLinkClient } from "better-auth/client/plugins";
import { createAuthClient } from "better-auth/react";
import {
  ConvexReactClient,
  type RequestForQueries,
  useConvexAuth,
  useConvexConnectionState,
  useMutation,
  usePaginatedQuery,
  useQueries,
  useQuery,
} from "convex/react";
import { useMemo, useRef, useState } from "react";
import type { AppEnv } from "../env.ts";
import { wipeLocalData } from "../lib/kv.ts";
import { undoOptimisticUpdate } from "../mail/undoSnapshots.ts";
import { discoverImap } from "../onboarding/imapDiscovery.ts";
import type {
  AccountId,
  ApiService,
  Backend,
  BillingInfo,
  DraftMeta,
  LabelView,
  ListResult,
  MailAccountView,
  Mutations,
  SearchArgs,
  SearchResult,
  ThreadDetail,
  ThreadId,
  ThreadRow,
} from "./types.ts";

const PAGE = 50;

export function makeAuthClient(siteUrl: string) {
  return createAuthClient({
    baseURL: siteUrl,
    plugins: [convexClient(), crossDomainClient(), magicLinkClient(), emailOTPClient()],
  });
}

type AuthClient = ReturnType<typeof makeAuthClient>;
// The provider's prop type is narrower than a client with the magic-link and email-OTP plugins; the runtime
// shape is what it needs (useSession, convex.token, crossDomain.oneTimeToken).
type ProviderAuthClient = Parameters<typeof ConvexBetterAuthProvider>[0]["authClient"];

/** Throws the Better Auth `{ error }` result as a WireError-shaped error with a §15 code. */
function check<T extends { error?: { message?: string; status?: number; code?: string } | null }>(r: T): T {
  if (r.error) {
    const code =
      r.error.status === 429
        ? "rate_limited"
        : r.error.status === 401
          ? "unauthenticated"
          : "invalid_argument";
    throw WireError.of(code, r.error.message || undefined);
  }
  return r;
}

function mapStatus(
  s: "LoadingFirstPage" | "CanLoadMore" | "LoadingMore" | "Exhausted",
): ListResult["status"] {
  return s === "LoadingFirstPage"
    ? "loading"
    : s === "CanLoadMore"
      ? "more"
      : s === "LoadingMore"
        ? "loading_more"
        : "done";
}

export function createConvexBackend(env: AppEnv): Backend {
  if (!env.convexUrl || !env.convexSiteUrl)
    throw new Error("VITE_CONVEX_URL and VITE_CONVEX_SITE_URL are required.");
  const convex = new ConvexReactClient(env.convexUrl, { expectAuth: true });
  const authClient: AuthClient = makeAuthClient(env.convexSiteUrl);
  // A function source, not the client itself: Better Auth's client is a Proxy that answers `in` checks for any
  // name, so `setAccessTokenSource(authClient)` would mistake it for a ready token source (D-269).
  setAccessTokenSource(async () => {
    const r = await authClient.convex.token({ fetchOptions: { throw: false } });
    const token = (r as { data?: { token?: unknown } | null }).data?.token;
    return typeof token === "string" ? token : null;
  });
  const syncUrl = env.syncUrl;
  const worker = syncUrl ? createWorkerClient({ baseUrl: syncUrl }) : null;
  const needWorker = () => {
    if (!worker) throw WireError.of("unreachable", "The sync worker URL is not configured.");
    return worker;
  };
  const apiClient = env.apiUrl ? createApiClient({ baseUrl: env.apiUrl }) : null;
  const needApi = () => {
    if (!apiClient) throw WireError.of("unreachable", "The Akatsuki API URL is not configured.");
    return apiClient;
  };
  const api$: ApiService = {
    configured: apiClient !== null,
    streamFeature: (feature, body, signal) => needApi().streamFeature(feature, body, { signal }),
    transcribe: (req, signal) => needApi().transcribe(req, { signal }),
    quota: () => needApi().quota(),
    checkout: () => needApi().checkout(),
    portal: () => needApi().portal(),
    cancel: (body) => needApi().cancel(body),
  };

  // The latest accounts and labels, for optimistic updates (folder providers need their exclusive labels).
  const known = { accounts: [] as MailAccountView[], labels: [] as LabelView[] };
  const contextFor = (accountId: string): ApplyContext => {
    const a = known.accounts.find((x) => x._id === accountId);
    const exclusive = known.labels
      .filter((l) => l.accountId === accountId && l.exclusive && l.kind !== "system")
      .map((l) => l._id);
    return applyContextFor(a?.provider ?? "gmail", exclusive);
  };
  const snoozeActionFor = (accountId: string): ThreadAction => {
    const a = known.accounts.find((x) => x._id === accountId);
    return a && a.provider !== "gmail"
      ? { kind: "move", to: SYS.snoozed }
      : { kind: "modify", add: [SYS.snoozed], remove: [SYS.inbox] };
  };

  function Provider({ children }: { children: React.ReactNode }) {
    return (
      <ConvexBetterAuthProvider client={convex} authClient={authClient as unknown as ProviderAuthClient}>
        {children}
      </ConvexBetterAuthProvider>
    );
  }

  return {
    kind: "convex",
    Provider,
    loginProviders: env.loginProviders,
    auth: {
      async sendMagicLink(email, callbackURL) {
        check(await authClient.signIn.magicLink({ email, callbackURL }));
      },
      async sendOtp(email) {
        check(await authClient.emailOtp.sendVerificationOtp({ email, type: "sign-in" }));
      },
      async verifyOtp(email, otp) {
        check(await authClient.signIn.emailOtp({ email, otp }));
      },
      async signInSocial(provider, callbackURL) {
        check(await authClient.signIn.social({ provider, callbackURL }));
      },
      async signOut() {
        clearAccessToken();
        await authClient.signOut().catch(() => undefined);
        // Sign-out wipes bodies and the outbox on this device (§16.1); a reload drops every memory cache.
        await wipeLocalData();
        window.location.replace("/signin");
      },
      async listSessions() {
        const [list, current] = await Promise.all([
          authClient.listSessions(),
          authClient.getSession().catch(() => null),
        ]);
        const currentToken = (current as { data?: { session?: { token?: string } } | null } | null)?.data
          ?.session?.token;
        const rows = (check(list).data ?? []) as {
          id: string;
          token: string;
          userAgent?: string | null;
          createdAt: Date | string;
          expiresAt: Date | string;
        }[];
        return rows.map((r) => ({
          id: r.id,
          token: r.token,
          userAgent: r.userAgent ?? null,
          createdAt: new Date(r.createdAt).getTime(),
          expiresAt: new Date(r.expiresAt).getTime(),
          current: r.token === currentToken,
        }));
      },
      async revokeSession(token) {
        check(await authClient.revokeSession({ token }));
      },
      async deleteUser() {
        check(await authClient.deleteUser({}));
      },
    },
    worker: {
      threadBodies: (threadId, signal) => needWorker().threadBodies(threadId, { signal }),
      getMessage: (messageId, signal) => needWorker().getMessage(messageId, { signal }),
      async getPartBlob(messageId, partId, signal) {
        return (await needWorker().getPart(messageId, partId, { signal })).blob();
      },
      async createLink(messageId, partId) {
        return (await needWorker().createLink(messageId, partId)).url;
      },
      async connectStart(provider, body) {
        return (await needWorker().connectStart(provider, body)).authorizeUrl;
      },
      async connectComplete(grant) {
        const r = await needWorker().connectComplete({ grant });
        return { accountId: r.accountId as AccountId, reconnected: r.reconnected };
      },
      async connectImap(body) {
        return (await needWorker().connectImap(body)).accountId;
      },
      discoverImap(email, signal) {
        const w = worker;
        return discoverImap(email, w ? () => w.discoverImap(email, { signal }) : null);
      },
      async search(accountId, q, signal) {
        const ids: ThreadId[] = [];
        let cursor: string | undefined;
        do {
          const r = await needWorker().search({ accountId, q, ...(cursor && { cursor }) }, { signal });
          ids.push(...(r.threadIds as ThreadId[]));
          cursor = r.nextCursor ?? undefined;
        } while (cursor && ids.length < 100);
        return ids;
      },
      syncAccount: (accountId) => needWorker().syncAccount(accountId),
      upload: (file, filename, signal) => needWorker().upload(file, filename, { signal }),
      putDraft: (clientDraftId, body) => needWorker().putDraft(clientDraftId, body),
      adoptDraft: (clientDraftId, messageId) => needWorker().adoptDraft(clientDraftId, messageId),
      deleteDraft: (clientDraftId) => needWorker().deleteDraft(clientDraftId),
      send: (body) => needWorker().send(body),
    },
    api: api$,
    async fetchThread(threadId) {
      try {
        return (await convex.query(api.mail.getThread, { threadId })) as ThreadDetail;
      } catch {
        return null;
      }
    },
    async fetchDraft(clientDraftId) {
      try {
        return (await convex.query(api.drafts.get, { clientDraftId })) as DraftMeta | null;
      } catch {
        return null;
      }
    },
    useAuthState() {
      const { isLoading, isAuthenticated } = useConvexAuth();
      const session = authClient.useSession();
      if (isLoading || (session.isPending && !session.data)) return { status: "loading" };
      const user = session.data?.user;
      if (!isAuthenticated || !user) return { status: "signed_out" };
      return { status: "signed_in", user: { id: user.id, email: user.email, name: user.name } };
    },
    useMe() {
      return useQuery(api.me.get, {});
    },
    useAccounts() {
      const accounts = useQuery(api.accounts.list, {});
      if (accounts) known.accounts = accounts;
      return accounts;
    },
    useLabels() {
      const labels = useQuery(api.labels.list, {});
      if (labels) known.labels = labels;
      return labels;
    },
    useSplits() {
      return useQuery(api.splits.list, {});
    },
    useThreadList(view) {
      const r = usePaginatedQuery(api.mail.listThreads, view ? { view } : "skip", { initialNumItems: PAGE });
      return { rows: r.results, status: view ? mapStatus(r.status) : "done", loadMore: r.loadMore };
    },
    useThreadRows(ids) {
      const requests = useMemo(
        (): RequestForQueries =>
          Object.fromEntries(ids.map((id) => [id, { query: api.mail.getThread, args: { threadId: id } }])),
        [ids],
      );
      const results = useQueries(requests);
      if (ids.some((id) => results[id] === undefined)) return undefined;
      return ids
        .map((id) => results[id])
        .filter((r): r is ThreadDetail => r !== null && typeof r === "object" && !(r instanceof Error))
        .map((r) => r.thread as ThreadRow);
    },
    useThread(threadId) {
      // useQueries returns errors instead of throwing: a thread deleted (or not owned, §3.3) while open
      // becomes `null` and the pane shows "That no longer exists." instead of crashing the shell.
      const requests = useMemo(
        (): RequestForQueries => (threadId ? { t: { query: api.mail.getThread, args: { threadId } } } : {}),
        [threadId],
      );
      const r = useQueries(requests).t as ThreadDetail | Error | undefined;
      if (!threadId) return null;
      if (r instanceof Error) return null;
      return r;
    },
    useUnreadCounts(labelKeys, accountId) {
      const keys = useMemo(() => [...labelKeys].slice(0, 30), [labelKeys]);
      return useQuery(
        api.mail.unreadCounts,
        keys.length ? { labelKeys: keys, ...(accountId && { accountId }) } : "skip",
      );
    },
    useSearch(args: SearchArgs | null): SearchResult {
      // Plain query pages (not usePaginatedQuery) so the first page's `unsupported` list is available.
      const key = args ? JSON.stringify(args) : "";
      const [pages, setPages] = useState<{ key: string; cursors: (string | null)[] }>({
        key,
        cursors: [null],
      });
      const cursors = pages.key === key ? pages.cursors : [null];
      const requests = useMemo((): RequestForQueries => {
        if (!args) return {};
        return Object.fromEntries(
          cursors.map((cursor, i) => [
            String(i),
            { query: api.search.threads, args: { ...args, paginationOpts: { numItems: PAGE, cursor } } },
          ]),
        );
      }, [args, cursors]);
      const results = useQueries(requests) as Record<string, unknown>;
      const lastRef = useRef<SearchResult | null>(null);
      if (!args) return { rows: [], status: "done", loadMore: () => {}, unsupported: [] };
      const rows: ThreadRow[] = [];
      let status: SearchResult["status"] = "done";
      let unsupported: string[] = [];
      let next: string | null = null;
      for (let i = 0; i < cursors.length; i++) {
        const r = results[String(i)];
        if (r instanceof Error) throw r;
        if (r === undefined) {
          status = i === 0 ? "loading" : "loading_more";
          break;
        }
        const page = r as {
          page: ThreadRow[];
          isDone: boolean;
          continueCursor: string;
          unsupported: string[];
        };
        if (i === 0) unsupported = page.unsupported;
        rows.push(...page.page);
        next = page.isDone ? null : page.continueCursor;
        status = page.isDone ? "done" : "more";
      }
      // Keep showing the previous results while a refined query loads (no flash of empty).
      if (status === "loading" && lastRef.current) return { ...lastRef.current, status: "loading" };
      const result: SearchResult = {
        rows,
        status,
        unsupported,
        loadMore: () => {
          if (next) setPages({ key, cursors: [...cursors, next] });
        },
      };
      lastRef.current = result;
      return result;
    },
    useTimers(kind) {
      return useQuery(api.mail.listTimers, { kind });
    },
    useOpFailures() {
      return useQuery(api.ops.failures, {});
    },
    useSenderPref(sender) {
      const r = useQuery(api.senderPrefs.get, sender ? { sender } : "skip");
      if (!sender) return null;
      if (r === undefined) return undefined;
      return r ? { allowRemoteImages: r.allowRemoteImages, vip: r.vip } : null;
    },
    useContacts(q) {
      return useQuery(api.contacts.search, q === null ? "skip" : { q, limit: 6 });
    },
    useMutations(): Mutations {
      const apply = useMutation(api.mail.apply).withOptimisticUpdate(applyOptimisticUpdate({ contextFor }));
      const undo = useMutation(api.mail.undo).withOptimisticUpdate(undoOptimisticUpdate(contextFor));
      const snooze = useMutation(api.mail.snooze).withOptimisticUpdate(
        snoozeOptimisticUpdate({ contextFor, snoozeActionFor }),
      );
      const unsnooze = useMutation(api.mail.unsnooze);
      const setReminder = useMutation(api.mail.setReminder);
      const bootstrap = useMutation(api.me.bootstrap);
      const updatePrefs = useMutation(api.me.updatePrefs).withOptimisticUpdate((store, { patch }) => {
        const me = store.getQuery(api.me.get, {});
        if (me)
          store.setQuery(
            api.me.get,
            {},
            { ...me, profile: { ...me.profile, prefs: { ...me.profile.prefs, ...patch } } },
          );
      });
      const updateProfile = useMutation(api.me.updateProfile);
      const ackFailure = useMutation(api.ops.ackFailure).withOptimisticUpdate((store, { opIds }) => {
        const cur = store.getQuery(api.ops.failures, {});
        if (cur)
          store.setQuery(
            api.ops.failures,
            {},
            cur.filter((f) => !opIds.includes(f._id)),
          );
      });
      const retryOp = useMutation(api.ops.retry);
      const setSenderPref = useMutation(api.senderPrefs.set);
      const registerDevice = useMutation(api.devices.register);
      const cancelReminder = useMutation(api.mail.cancelReminder);
      const discardDraft = useMutation(api.drafts.discard);
      const cancelSend = useMutation(api.sends.cancel);
      const rescheduleSend = useMutation(api.sends.reschedule);
      const resolveUnknownSend = useMutation(api.sends.resolveUnknown);
      const saveSplit = useMutation(api.splits.save);
      const removeSplit = useMutation(api.splits.remove);
      const reorderSplits = useMutation(api.splits.reorder).withOptimisticUpdate((store, { order }) => {
        const cur = store.getQuery(api.splits.list, {});
        if (cur)
          store.setQuery(
            api.splits.list,
            {},
            cur.map((s) => ({ ...s, order: Math.max(0, order.indexOf(s._id)) })),
          );
      });
      const saveRule = useMutation(api.rules.save);
      const removeRule = useMutation(api.rules.remove);
      const saveSnippet = useMutation(api.snippets.save);
      const removeSnippet = useMutation(api.snippets.remove);
      const markSnippetUsed = useMutation(api.snippets.markUsed);
      const createLabel = useMutation(api.labels.create);
      const renameLabel = useMutation(api.labels.rename).withOptimisticUpdate((store, { labelId, name }) => {
        const cur = store.getQuery(api.labels.list, {});
        if (cur)
          store.setQuery(
            api.labels.list,
            {},
            cur.map((l) => (l._id === labelId ? { ...l, name } : l)),
          );
      });
      const removeLabel = useMutation(api.labels.remove).withOptimisticUpdate((store, { labelId }) => {
        const cur = store.getQuery(api.labels.list, {});
        if (cur)
          store.setQuery(
            api.labels.list,
            {},
            cur.filter((l) => l._id !== labelId),
          );
      });
      const updateIdentity = useMutation(api.identities.update);
      const updateAccount = useMutation(api.accounts.update);
      const setBackfillHorizon = useMutation(api.accounts.setBackfillHorizon);
      const removeAccount = useMutation(api.accounts.remove);
      const saveAiSettings = useMutation(api.ai.saveSettings).withOptimisticUpdate((store, patch) => {
        const cur = store.getQuery(api.ai.getSettings, {});
        if (!cur) return;
        const { voiceProfile, ...rest } = patch;
        const next = { ...cur, ...rest };
        if (voiceProfile === null) delete (next as { voiceProfile?: string }).voiceProfile;
        else if (voiceProfile !== undefined) next.voiceProfile = voiceProfile;
        store.setQuery(api.ai.getSettings, {}, next);
      });
      const removeDevice = useMutation(api.devices.remove);
      const deleteAccount = useMutation(api.me.deleteAccount);
      const claimTriage = useMutation(api.ai.claimTriage);
      const setTriage = useMutation(api.ai.setTriage);
      return useMemo(
        (): Mutations => ({
          apply: (a) => apply(a),
          undo: (a) => undo(a),
          snooze: (a) => snooze(a),
          unsnooze: (a) => unsnooze(a),
          setReminder: (a) => setReminder(a),
          bootstrap: (a) => bootstrap(a),
          updatePrefs: (a) => updatePrefs(a),
          updateProfile: (a) => updateProfile(a),
          ackFailure: (a) => ackFailure(a),
          retryOp: (a) => retryOp(a),
          setSenderPref: (a) => setSenderPref(a),
          registerDevice: (a) => registerDevice(a),
          cancelReminder: (a) => cancelReminder(a),
          discardDraft: (a) => discardDraft(a),
          cancelSend: (a) => cancelSend(a),
          rescheduleSend: (a) => rescheduleSend(a),
          resolveUnknownSend: (a) => resolveUnknownSend(a),
          saveSplit: (a) => saveSplit(a),
          removeSplit: (a) => removeSplit(a),
          reorderSplits: (a) => reorderSplits(a),
          saveRule: (a) => saveRule(a),
          removeRule: (a) => removeRule(a),
          saveSnippet: (a) => saveSnippet(a),
          removeSnippet: (a) => removeSnippet(a),
          markSnippetUsed: (a) => markSnippetUsed(a),
          createLabel: (a) => createLabel(a),
          renameLabel: (a) => renameLabel(a),
          removeLabel: (a) => removeLabel(a),
          updateIdentity: (a) => updateIdentity(a),
          updateAccount: (a) => updateAccount(a),
          setBackfillHorizon: (a) => setBackfillHorizon(a),
          removeAccount: (a) => removeAccount(a),
          saveAiSettings: (a) => saveAiSettings(a),
          removeDevice: (a) => removeDevice(a),
          deleteAccount: (a) => deleteAccount(a),
          claimTriage: (a) => claimTriage(a),
          setTriage: (a) => setTriage(a),
        }),
        [
          apply,
          undo,
          snooze,
          unsnooze,
          setReminder,
          bootstrap,
          updatePrefs,
          updateProfile,
          ackFailure,
          retryOp,
          setSenderPref,
          registerDevice,
          cancelReminder,
          discardDraft,
          cancelSend,
          rescheduleSend,
          resolveUnknownSend,
          saveSplit,
          removeSplit,
          reorderSplits,
          saveRule,
          removeRule,
          saveSnippet,
          removeSnippet,
          markSnippetUsed,
          createLabel,
          renameLabel,
          removeLabel,
          updateIdentity,
          updateAccount,
          setBackfillHorizon,
          removeAccount,
          saveAiSettings,
          removeDevice,
          deleteAccount,
          claimTriage,
          setTriage,
        ],
      );
    },
    useConnected() {
      const s = useConvexConnectionState();
      return !s.hasEverConnected || s.isWebSocketConnected;
    },
    useIdentities(accountId) {
      return useQuery(api.identities.list, accountId ? { accountId } : {});
    },
    useSnippets() {
      return useQuery(api.snippets.list, {});
    },
    useRules() {
      return useQuery(api.rules.list, {});
    },
    useDevices() {
      return useQuery(api.devices.list, {});
    },
    useAiSettings() {
      return useQuery(api.ai.getSettings, {});
    },
    useBilling() {
      return useQuery(api.ee.billing.entitlement, {}) as BillingInfo | undefined;
    },
    useSends(states) {
      const key = states?.join(",") ?? "";
      // biome-ignore lint/correctness/useExhaustiveDependencies: `key` stands for `states`
      const args = useMemo(() => (states ? { states } : {}), [key]);
      return useQuery(api.sends.list, args);
    },
    useDrafts() {
      const r = useQuery(api.drafts.list, { paginationOpts: { numItems: PAGE, cursor: null } });
      return r?.page;
    },
    useReadReceipt(messageId) {
      const r = useQuery(api.readReceipts.forMessage, messageId ? { messageId } : "skip");
      return messageId ? r : null;
    },
    useSenderPrefFull(sender) {
      const r = useQuery(api.senderPrefs.get, sender ? { sender } : "skip");
      return sender ? r : null;
    },
  };
}

export type { AccountId };
