// Network clients: Convex (live queries and mutations), the sync worker (bodies, drafts, send; §7) and apps/api
// (managed AI; §8). Worker and api calls carry the user's Convex JWT from `getAccessToken()` (packages/wire).
// `expo/fetch` is used for the worker because it streams response bodies (NDJSON thread bodies, §7.2).
import { type ApiClient, createApiClient, createWorkerClient, type WorkerClient } from "@akatsuki/wire";
import { ConvexReactClient } from "convex/react";
import { fetch as expoFetch } from "expo/fetch";
import { ENV } from "../env";

export const convex: ConvexReactClient | null = ENV.env
  ? new ConvexReactClient(ENV.env.convexUrl, { unsavedChangesWarning: false, expectAuth: true })
  : null;

const streamingFetch = expoFetch as unknown as typeof fetch;

export const worker: WorkerClient | null = ENV.env
  ? createWorkerClient({ baseUrl: ENV.env.syncUrl, fetch: streamingFetch })
  : null;

export const api: ApiClient | null = ENV.env
  ? createApiClient({ baseUrl: ENV.env.apiUrl, fetch: streamingFetch })
  : null;

export function requireWorker(): WorkerClient {
  if (!worker) throw new Error("The sync worker URL is not configured.");
  return worker;
}
