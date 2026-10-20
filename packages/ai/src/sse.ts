// AASP (SSE) reader for the managed path. The parser is @akatsuki/wire's (src/aasp.ts), the TypeScript twin of
// crates/core::wire::aasp, which runs the shared fixtures in packages/wire/fixtures/aasp (D-251).
export { type AaspFrameResult, AaspLineParser, parseAaspStream } from "@akatsuki/wire/aasp";
