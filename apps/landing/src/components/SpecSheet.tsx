import { Nb } from "./Nb";

// §5.6: where your mail lives. Every row is a product fact from README.md and docs/PRIVACY.md.
export const SPEC_ROWS: [string, string, string?][] = [
  [
    "Message bodies",
    "Fetched from your provider when you open them, cached on your device.",
    "Never stored on Akatsuki’s servers.",
  ],
  ["Attachments", "Handled the same way as bodies."],
  [
    "Headers and snippets",
    "Sender, recipients, subject, date and up to 200 characters.",
    "Kept so lists and search are instant.",
  ],
  ["Mailbox credentials", "Held only by the sync worker, encrypted at rest.", "They never reach an app."],
  ["Email HTML", "Sanitised before it’s shown. Scripts never run."],
  [
    "Remote images",
    "Blocked until you allow them.",
    "Then loaded through a proxy that hides your IP address.",
  ],
  ["Read receipts", "Off unless you turn them on."],
  ["Your API keys", "Stay on the device you typed them into."],
  ["Training and ads", "Your mail is never used to train models, never sold and never used for advertising."],
  ["This website", "No analytics, no cookies, no third-party requests."],
  [
    "Source",
    "Open source, free for personal use, under ASAL-1.0.",
    "Commercial use, including by any organisation, needs written permission.",
  ],
];

/** Rows that are about the site or the licence, not about where mail lives. */
const SITE_ROWS = new Set(["This website", "Source"]);

export function SpecSheet({ mailOnly = false }: { mailOnly?: boolean }) {
  const rows = mailOnly ? SPEC_ROWS.filter(([k]) => !SITE_ROWS.has(k)) : SPEC_ROWS;
  return (
    <dl className="spec">
      {rows.map(([k, v, more]) => (
        <div key={k} className="spec-row">
          <dt>{k}</dt>
          <dd>
            <Nb>{v}</Nb>
            {more && (
              <span className="spec-more">
                <Nb>{more}</Nb>
              </span>
            )}
          </dd>
        </div>
      ))}
    </dl>
  );
}
