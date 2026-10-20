/* eslint-disable */
/**
 * Generated `api` utility.
 *
 * THIS CODE IS AUTOMATICALLY GENERATED.
 *
 * To regenerate, run `npx convex dev`.
 * @module
 */

import type * as accounts from "../accounts.js";
import type * as ai from "../ai.js";
import type * as auth from "../auth.js";
import type * as authLimits from "../authLimits.js";
import type * as billing from "../billing.js";
import type * as contacts from "../contacts.js";
import type * as crons from "../crons.js";
import type * as devices from "../devices.js";
import type * as drafts from "../drafts.js";
import type * as ee_billing from "../ee/billing.js";
import type * as ee_entitlement from "../ee/entitlement.js";
import type * as ee_service from "../ee/service.js";
import type * as email from "../email.js";
import type * as http from "../http.js";
import type * as identities from "../identities.js";
import type * as labels from "../labels.js";
import type * as lib_auth from "../lib/auth.js";
import type * as lib_billing from "../lib/billing.js";
import type * as lib_email from "../lib/email.js";
import type * as lib_errors from "../lib/errors.js";
import type * as lib_model from "../lib/model.js";
import type * as lib_ops from "../lib/ops.js";
import type * as lib_query from "../lib/query.js";
import type * as lib_rateLimits from "../lib/rateLimits.js";
import type * as lib_revs from "../lib/revs.js";
import type * as lib_rows from "../lib/rows.js";
import type * as lib_sanitizeUserHtml from "../lib/sanitizeUserHtml.js";
import type * as lib_shards from "../lib/shards.js";
import type * as lib_threads from "../lib/threads.js";
import type * as lib_timers from "../lib/timers.js";
import type * as lib_userRows from "../lib/userRows.js";
import type * as lib_validators from "../lib/validators.js";
import type * as lib_workerSync from "../lib/workerSync.js";
import type * as mail from "../mail.js";
import type * as me from "../me.js";
import type * as mobileAuth from "../mobileAuth.js";
import type * as notify from "../notify.js";
import type * as ops from "../ops.js";
import type * as readReceipts from "../readReceipts.js";
import type * as rules from "../rules.js";
import type * as search from "../search.js";
import type * as senderPrefs from "../senderPrefs.js";
import type * as sends from "../sends.js";
import type * as snippets from "../snippets.js";
import type * as splits from "../splits.js";
import type * as sync from "../sync.js";
import type * as timers from "../timers.js";
import type * as worker from "../worker.js";

import type {
  ApiFromModules,
  FilterApi,
  FunctionReference,
} from "convex/server";

declare const fullApi: ApiFromModules<{
  accounts: typeof accounts;
  ai: typeof ai;
  auth: typeof auth;
  authLimits: typeof authLimits;
  billing: typeof billing;
  contacts: typeof contacts;
  crons: typeof crons;
  devices: typeof devices;
  drafts: typeof drafts;
  "ee/billing": typeof ee_billing;
  "ee/entitlement": typeof ee_entitlement;
  "ee/service": typeof ee_service;
  email: typeof email;
  http: typeof http;
  identities: typeof identities;
  labels: typeof labels;
  "lib/auth": typeof lib_auth;
  "lib/billing": typeof lib_billing;
  "lib/email": typeof lib_email;
  "lib/errors": typeof lib_errors;
  "lib/model": typeof lib_model;
  "lib/ops": typeof lib_ops;
  "lib/query": typeof lib_query;
  "lib/rateLimits": typeof lib_rateLimits;
  "lib/revs": typeof lib_revs;
  "lib/rows": typeof lib_rows;
  "lib/sanitizeUserHtml": typeof lib_sanitizeUserHtml;
  "lib/shards": typeof lib_shards;
  "lib/threads": typeof lib_threads;
  "lib/timers": typeof lib_timers;
  "lib/userRows": typeof lib_userRows;
  "lib/validators": typeof lib_validators;
  "lib/workerSync": typeof lib_workerSync;
  mail: typeof mail;
  me: typeof me;
  mobileAuth: typeof mobileAuth;
  notify: typeof notify;
  ops: typeof ops;
  readReceipts: typeof readReceipts;
  rules: typeof rules;
  search: typeof search;
  senderPrefs: typeof senderPrefs;
  sends: typeof sends;
  snippets: typeof snippets;
  splits: typeof splits;
  sync: typeof sync;
  timers: typeof timers;
  worker: typeof worker;
}>;

/**
 * A utility for referencing Convex functions in your app's public API.
 *
 * Usage:
 * ```js
 * const myFunctionReference = api.myModule.myFunction;
 * ```
 */
export declare const api: FilterApi<
  typeof fullApi,
  FunctionReference<any, "public">
>;

/**
 * A utility for referencing Convex functions in your app's internal API.
 *
 * Usage:
 * ```js
 * const myFunctionReference = internal.myModule.myFunction;
 * ```
 */
export declare const internal: FilterApi<
  typeof fullApi,
  FunctionReference<any, "internal">
>;

export declare const components: {
  betterAuth: import("@convex-dev/better-auth/_generated/component.js").ComponentApi<"betterAuth">;
  rateLimiter: import("@convex-dev/rate-limiter/_generated/component.js").ComponentApi<"rateLimiter">;
};
