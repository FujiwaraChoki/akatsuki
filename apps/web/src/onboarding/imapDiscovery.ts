// IMAP/SMTP host autodetect for the "Connect IMAP" form (§6.7 step 5, D-264, D-396).
//
// The worker owns discovery: `GET /v1/connect/imap/discover?email=` answers from `crates/providers::imap::autoconfig`
// (a built-in table, then the domain's Mozilla autoconfig file, then the Thunderbird ISPDB, all fetched through its
// SSRF-guarded client). When the worker knows nothing or cannot be reached, the form falls back to the conventional
// `imap.<domain>` / `smtp.<domain>` guess, which the user can edit before connecting.
import type { ImapDiscovery, ImapEndpoint } from "../backend/types.ts";

const imp = (host: string, port = 993): ImapEndpoint => ({ host, port, tls: "implicit" });
const start = (host: string): ImapEndpoint => ({ host, port: 587, tls: "starttls" });

/** The lowercased domain of a plausible public address, as `email_domain` in the Rust module. */
export function emailDomain(email: string): string | null {
  const at = email.trim().lastIndexOf("@");
  if (at <= 0) return null;
  const d = email
    .trim()
    .slice(at + 1)
    .replace(/\.+$/, "")
    .toLowerCase();
  if (!d || d.length > 253 || !d.includes(".")) return null;
  if (/^[0-9.]+$/.test(d) || d.includes(":")) return null; // IP literals
  if (d === "localhost" || /\.(localhost|local|internal)$/.test(d)) return null;
  const ok = d
    .split(".")
    .every(
      (l) =>
        l.length > 0 && l.length <= 63 && /^[a-z0-9-]+$/.test(l) && !l.startsWith("-") && !l.endsWith("-"),
    );
  return ok ? d : null;
}

/** Last resort: `imap.<domain>:993` and `smtp.<domain>:587` (STARTTLS), marked as a guess. */
export function guess(email: string): ImapDiscovery | null {
  const d = emailDomain(email);
  if (!d) return null;
  return {
    imap: imp(`imap.${d}`),
    smtp: start(`smtp.${d}`),
    username: email.trim(),
    source: "guess",
    appPasswordRequired: false,
  };
}

function isEndpoint(v: unknown): v is ImapEndpoint {
  const e = v as ImapEndpoint;
  return (
    typeof e === "object" &&
    e !== null &&
    typeof e.host === "string" &&
    e.host.length > 0 &&
    Number.isInteger(e.port) &&
    e.port > 0 &&
    e.port < 65536 &&
    (e.tls === "implicit" || e.tls === "starttls")
  );
}

/** Validates the worker's JSON (`Discovered` in the Rust module, camelCase). */
export function parseDiscovery(v: unknown): ImapDiscovery | null {
  const d = v as Partial<ImapDiscovery>;
  if (typeof d !== "object" || d === null || !isEndpoint(d.imap) || !isEndpoint(d.smtp)) return null;
  const source =
    d.source === "autoconfig" || d.source === "ispdb" || d.source === "well_known" ? d.source : "well_known";
  return {
    imap: d.imap,
    smtp: d.smtp,
    username: typeof d.username === "string" && d.username ? d.username : "",
    source,
    appPasswordRequired: d.appPasswordRequired === true,
  };
}

/** Worker first (`ask` performs the request and returns its JSON, or throws), then the guess. Never throws. */
export async function discoverImap(
  email: string,
  ask: (() => Promise<unknown>) | null,
): Promise<ImapDiscovery | null> {
  if (!emailDomain(email)) return null;
  if (ask) {
    try {
      const found = parseDiscovery(await ask());
      if (found) return { ...found, username: found.username || email.trim() };
    } catch {
      /* nothing known (404), offline, rate limited or not entitled: fall back */
    }
  }
  return guess(email);
}
