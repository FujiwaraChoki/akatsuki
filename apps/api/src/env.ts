// Hono context variables shared by middleware and routes.
import type { LogMeta } from "./log.ts";

export type AppEnv = {
  Variables: {
    userId?: string;
    userEmail?: string;
    userToken?: string;
    workerSub?: string;
    logMeta?: LogMeta;
    errorCode?: string;
    /** Settles when a streamed response finishes (the log line waits for token counts). */
    streamDone?: Promise<void>;
  };
};
