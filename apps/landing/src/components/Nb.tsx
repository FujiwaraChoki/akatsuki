// Keeps hyphenated terms whole ("ASAL-1.0", "non-commercial", "sign-in") without U+2011, which
// the brand fonts do not contain, and keeps the licence phrase "open source, free for personal use"
// on one line.
import { Fragment } from "react";

const KEEP = /(ASAL-1\.0|non-commercial|sign-in|[Oo]pen source, free for personal use)/;

export function Nb({ children }: { children: string }) {
  return (
    <>
      {children.split(KEEP).map((part, i) =>
        KEEP.test(part) ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: static text split at build time; the order never changes
          <span key={i} className="nowrap">
            {part}
          </span>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: static text split at build time; the order never changes
          <Fragment key={i}>{part}</Fragment>
        ),
      )}
    </>
  );
}

/** Display headings: wraps each word-internal hyphen so Instrument Serif's wide, low hyphen is
 *  tightened (see `.hy`), and keeps the hyphenated word on one line. */
export function Hy({ children }: { children: string }) {
  const words = children.split(/(\S*[A-Za-z]-[A-Za-z]\S*)/);
  return (
    <>
      {words.map((w, i) =>
        /[A-Za-z]-[A-Za-z]/.test(w) ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: static text split at build time; the order never changes
          <span key={i} className="nowrap">
            {w.split("-").map((part, j) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: static text split at build time; the order never changes
              <Fragment key={j}>
                {j > 0 && <span className="hy">-</span>}
                {part}
              </Fragment>
            ))}
          </span>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: static text split at build time; the order never changes
          <Fragment key={i}>{w}</Fragment>
        ),
      )}
    </>
  );
}
