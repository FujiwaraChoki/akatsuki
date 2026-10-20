// Akatsuki AI Stream Protocol (ARCHITECTURE §12.6). The canonical types, encoder and parser live in
// @akatsuki/wire (src/aasp.ts, D-251); this module re-exports them so the BYO path and apps/api emit exactly
// the events the clients parse.
export {
  AASP_CONTENT_TYPE,
  AASP_EVENTS,
  type AaspError,
  type AaspEvent,
  type AaspEventName,
  type AaspUsage,
  aaspReadableStream,
  encodeAaspEvent,
} from "@akatsuki/wire/aasp";
