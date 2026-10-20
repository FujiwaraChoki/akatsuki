// The signed-in user's mail context: profile and prefs, entitlement, accounts, labels and identities, plus the
// per-account `ApplyContext` and action helpers from packages/mail-model (§5.1). One provider for the app shell so
// screens share the same live queries.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import {
  type ApplyContext,
  actionsFor,
  applyContextFor,
  type Provider,
  type ThreadActions,
} from "@akatsuki/mail-model";
import { useQuery } from "convex/react";
import { getCalendars } from "expo-localization";
import { createContext, type ReactNode, useContext, useMemo } from "react";
import { safeZone } from "../vm/zoned";

type MeResult = NonNullable<(typeof api.me.get)["_returnType"]>;
type AccountView = (typeof api.accounts.list)["_returnType"][number];
type LabelView = (typeof api.labels.list)["_returnType"][number];
type IdentityView = (typeof api.identities.list)["_returnType"][number];

export type Prefs = MeResult["profile"]["prefs"];

export interface MailState {
  me: MeResult | null | undefined;
  prefs: Prefs | undefined;
  timeZone: string;
  entitled: boolean;
  accounts: AccountView[];
  labels: LabelView[];
  identities: IdentityView[];
  selfEmails: ReadonlySet<string>;
  accountById: Map<string, AccountView>;
  contextFor(accountId: string): ApplyContext;
  actionsFor(accountId: string): ThreadActions | null;
  /** The identity a new message is sent from: the default one of the first account. */
  defaultIdentity: IdentityView | undefined;
}

const Ctx = createContext<MailState | null>(null);
const NO_ACCOUNTS: AccountView[] = [];
const NO_LABELS: LabelView[] = [];
const NO_IDENTITIES: IdentityView[] = [];

export function deviceTimeZone(): string {
  try {
    return safeZone(getCalendars()[0]?.timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone);
  } catch {
    return "UTC";
  }
}

export function MailProvider({ children }: { children: ReactNode }) {
  const me = useQuery(api.me.get, {});
  const entitled = me?.entitlement.hostedSync === true;
  const accounts = useQuery(api.accounts.list, {}) ?? NO_ACCOUNTS;
  const labels = useQuery(api.labels.list, entitled ? {} : "skip") ?? NO_LABELS;
  const identities = useQuery(api.identities.list, entitled ? {} : "skip") ?? NO_IDENTITIES;

  const value = useMemo<MailState>(() => {
    const accountById = new Map(accounts.map((a) => [a._id as string, a]));
    const labelsByAccount = new Map<string, LabelView[]>();
    for (const l of labels) {
      const list = labelsByAccount.get(l.accountId) ?? [];
      list.push(l);
      labelsByAccount.set(l.accountId, list);
    }
    const contexts = new Map<string, ApplyContext>();
    const selfEmails = new Set<string>([
      ...accounts.map((a) => a.email.toLowerCase()),
      ...identities.map((i) => i.email.toLowerCase()),
    ]);
    const defaultIdentity =
      identities.find((i) => i.isDefault && i.accountId === accounts[0]?._id) ??
      identities.find((i) => i.isDefault) ??
      identities[0];
    return {
      me,
      prefs: me?.profile.prefs,
      timeZone: safeZone(me?.profile.timezone ?? deviceTimeZone()),
      entitled,
      accounts,
      labels,
      identities,
      selfEmails,
      accountById,
      defaultIdentity,
      contextFor(accountId) {
        let ctx = contexts.get(accountId);
        if (!ctx) {
          const account = accountById.get(accountId);
          const provider: Provider = account?.provider ?? "gmail";
          const exclusive = (labelsByAccount.get(accountId) ?? [])
            .filter((l) => l.exclusive)
            .map((l) => l._id as string);
          ctx = applyContextFor(provider, exclusive);
          contexts.set(accountId, ctx);
        }
        return ctx;
      },
      actionsFor(accountId) {
        const account = accountById.get(accountId);
        if (!account) return null;
        const ls = (labelsByAccount.get(accountId) ?? []).map((l) => ({
          _id: l._id as string,
          exclusive: l.exclusive,
          ...(l.role ? { role: l.role } : {}),
        }));
        return actionsFor({ provider: account.provider, caps: account.caps }, ls);
      },
    };
  }, [me, entitled, accounts, labels, identities]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useMail(): MailState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useMail outside MailProvider");
  return v;
}

export type { AccountView, Id, IdentityView, LabelView };
