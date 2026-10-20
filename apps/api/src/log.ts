// Strict request logging (ARCHITECTURE §8, §14.2): one JSON line per request with method, route template,
// status, latency, sha256(userId)[:12], model, token counts and error code. Never request or response
// bodies, query strings, paths with ids, or headers other than user-agent. test/log.test.ts enforces this.
import { createHash } from "node:crypto";
import type { MiddlewareHandler } from "hono";
import { routePath } from "hono/route";
import type { AppEnv } from "./env.ts";

export interface LogLine {
  t: string;
  method: string;
  route: string;
  status: number;
  ms: number;
  user?: string;
  model?: string;
  inTok?: number;
  outTok?: number;
  audioSec?: number;
  code?: string;
  ua?: string;
}

export type LogSink = (line: LogLine) => void;

export const stdoutSink: LogSink = (line) => {
  process.stdout.write(`${JSON.stringify(line)}\n`);
};

export const userHash = (userId: string) => createHash("sha256").update(userId).digest("hex").slice(0, 12);

/** Fields a handler may add to the log line. Anything else is dropped. */
export interface LogMeta {
  model?: string;
  inputTokens?: number;
  outputTokens?: number;
  audioSeconds?: number;
  code?: string;
}

const UA_MAX = 120;

export function requestLogger(sink: LogSink, now: () => number = Date.now): MiddlewareHandler<AppEnv> {
  return async (c, next) => {
    const start = now();
    let thrown: unknown;
    try {
      await next();
    } catch (e) {
      thrown = e;
    }
    const emit = () => {
      const meta = c.get("logMeta") ?? {};
      const userId = c.get("userId");
      const ua = c.req.header("user-agent");
      const line: LogLine = {
        t: new Date(start).toISOString(),
        method: c.req.method,
        // The matched route template (e.g. /v1/ai/:feature), never the concrete path or query string.
        route: routePath(c, -1),
        status: thrown ? 500 : c.res.status,
        ms: now() - start,
      };
      if (userId) line.user = userHash(userId);
      if (meta.model) line.model = meta.model.slice(0, 80);
      if (Number.isFinite(meta.inputTokens)) line.inTok = meta.inputTokens;
      if (Number.isFinite(meta.outputTokens)) line.outTok = meta.outputTokens;
      if (Number.isFinite(meta.audioSeconds)) line.audioSec = meta.audioSeconds;
      const code = meta.code ?? c.get("errorCode");
      if (code) line.code = code.slice(0, 40);
      if (ua) line.ua = ua.replace(/[^\x20-\x7e]/g, "").slice(0, UA_MAX);
      sink(line);
    };
    // Streaming routes register a promise that settles when the stream ends, so token counts are known.
    const pending = c.get("streamDone");
    if (pending && !thrown) void pending.then(emit, emit);
    else emit();
    if (thrown) throw thrown;
  };
}

/** Merges fields into the request's log meta. */
export function addLogMeta(
  c: { get: (k: "logMeta") => LogMeta | undefined; set: (k: "logMeta", v: LogMeta) => void },
  meta: LogMeta,
) {
  c.set("logMeta", { ...(c.get("logMeta") ?? {}), ...meta });
}
