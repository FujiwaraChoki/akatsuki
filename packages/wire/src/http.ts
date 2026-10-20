// Small fetch layer shared by the worker and apps/api clients: bearer auth from `getAccessToken()`, one retry
// with a fresh token after a 401, §15 error bodies turned into `WireError`, and NDJSON streaming.
import { type ErrorPayload, parseErrorPayload, WireError } from "./errors.ts";
import { getAccessToken as defaultGetAccessToken } from "./token.ts";

export interface HttpClientOptions {
  /** e.g. `https://sync.akatsuki.vc` or `https://api.akatsuki.vc` (no trailing slash needed). */
  baseUrl: string;
  /** Defaults to the process-wide `getAccessToken()` (configure it with `setAccessTokenSource`). */
  getAccessToken?: (opts?: { forceRefresh?: boolean }) => Promise<string>;
  /** Defaults to `globalThis.fetch`. */
  fetch?: typeof fetch;
}

export interface RequestOptions {
  json?: unknown;
  body?: BodyInit;
  headers?: Record<string, string>;
  query?: Record<string, string | undefined>;
  signal?: AbortSignal;
  /** Send the bearer token (default true). */
  auth?: boolean;
}

/** Thrown for network failures (no HTTP response at all). Retryable. */
export const NETWORK_ERROR: ErrorPayload = {
  code: "unreachable",
  message: "Could not reach the server.",
  retryable: true,
};

/** Builds `base + path + ?query`, dropping undefined query values. */
export function buildUrl(baseUrl: string, path: string, query?: Record<string, string | undefined>): string {
  const url = new URL(path.replace(/^\/+/, ""), baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`);
  for (const [k, v] of Object.entries(query ?? {})) if (v !== undefined) url.searchParams.set(k, v);
  return url.toString();
}

/** Encodes one path segment. */
export const seg = (s: string): string => encodeURIComponent(s);

/** Reads a non-2xx response into a WireError (status kept; `Retry-After` fills `retryAfterMs`). */
export async function errorFromResponse(res: Response): Promise<WireError> {
  let payload: ErrorPayload | null = null;
  try {
    payload = parseErrorPayload(await res.json());
  } catch {
    payload = null;
  }
  if (!payload) {
    const code = res.status === 401 ? "unauthenticated" : res.status === 404 ? "not_found" : "internal";
    payload = { code, message: "The server returned an unexpected response.", retryable: res.status >= 500 };
  }
  if (payload.retryAfterMs === undefined) {
    const ra = Number(res.headers.get("retry-after"));
    if (Number.isFinite(ra) && ra > 0) payload.retryAfterMs = ra * 1000;
  }
  return new WireError(payload, res.status);
}

export interface HttpClient {
  readonly baseUrl: string;
  /** Sends a request and returns the 2xx response; throws `WireError` otherwise. */
  request(method: string, path: string, opts?: RequestOptions): Promise<Response>;
  /** `request` and parse the JSON body. */
  json<T>(method: string, path: string, opts?: RequestOptions): Promise<T>;
  /** Absolute URL for a server-relative path (e.g. a `/v1/l/<token>` link). */
  url(path: string): string;
}

export function createHttpClient(opts: HttpClientOptions): HttpClient {
  const doFetch =
    opts.fetch ?? ((input: RequestInfo | URL, init?: RequestInit) => globalThis.fetch(input, init));
  const token = opts.getAccessToken ?? defaultGetAccessToken;

  async function send(
    method: string,
    path: string,
    o: RequestOptions,
    forceRefresh: boolean,
  ): Promise<Response> {
    const headers: Record<string, string> = { ...o.headers };
    if (o.auth !== false) headers.authorization = `Bearer ${await token({ forceRefresh })}`;
    let body = o.body;
    if (o.json !== undefined) {
      headers["content-type"] = "application/json";
      body = JSON.stringify(o.json);
    }
    try {
      return await doFetch(buildUrl(opts.baseUrl, path, o.query), {
        method,
        headers,
        body,
        signal: o.signal,
      });
    } catch (e) {
      if ((e as { name?: string }).name === "AbortError") throw e;
      throw new WireError({ ...NETWORK_ERROR });
    }
  }

  const client: HttpClient = {
    baseUrl: opts.baseUrl,
    async request(method, path, o = {}) {
      let res = await send(method, path, o, false);
      // One retry with a fresh token: the cached JWT may have been revoked or the clock skewed. Bodies
      // that are streams cannot be replayed, so they are not retried.
      const streamed = typeof ReadableStream !== "undefined" && o.body instanceof ReadableStream;
      if (res.status === 401 && o.auth !== false && !streamed) {
        res = await send(method, path, o, true);
      }
      if (!res.ok) throw await errorFromResponse(res);
      return res;
    },
    async json<T>(method: string, path: string, o?: RequestOptions) {
      const res = await client.request(method, path, o);
      return (await res.json()) as T;
    },
    url: (path) => buildUrl(opts.baseUrl, path),
  };
  return client;
}

/** Parses an `application/x-ndjson` body line by line (bodies stream newest first, §7.2). */
export async function* readNdjson<T>(body: ReadableStream<Uint8Array>): AsyncGenerator<T> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      buf += decoder.decode(value ?? new Uint8Array(), { stream: !done });
      let nl = buf.indexOf("\n");
      while (nl !== -1) {
        const line = buf.slice(0, nl).trim();
        buf = buf.slice(nl + 1);
        if (line !== "") yield JSON.parse(line) as T;
        nl = buf.indexOf("\n");
      }
      if (done) break;
    }
    if (buf.trim() !== "") yield JSON.parse(buf) as T;
  } finally {
    reader.releaseLock();
  }
}
