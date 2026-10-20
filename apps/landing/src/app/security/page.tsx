import "../doc.css";
import type { Metadata } from "next";
import { DocLayout, type DocPart } from "@/components/DocLayout";
import { readContent, readMinutes, renderDoc } from "@/lib/markdown";
import { mailto, SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Security",
  description: "How Akatsuki protects your mail, and how to report a vulnerability privately.",
  alternates: { canonical: "/security" },
};

const MODEL = [
  [
    "Hostile HTML, handled",
    "One sanitiser for every app removes scripts, forms, event handlers and tracking pixels. Messages render in a sandbox where no script can run.",
  ],
  [
    "Images on your terms",
    "Remote images are blocked by default. When you allow them, they load through a proxy that hides your IP address from the sender.",
  ],
  [
    "Credentials sealed",
    "Mailbox tokens and passwords are encrypted with XChaCha20-Poly1305 before they reach the database. Only the sync server holds the key.",
  ],
  [
    "No message text at rest",
    "Message bodies and attachments are never stored on our servers. Logs record ids, counts and error codes, never message text.",
  ],
];

export default function SecurityPage() {
  const doc = renderDoc(readContent("security"));
  const parts: DocPart[] = [
    {
      id: "how-akatsuki-protects-your-mail",
      num: "01",
      title: "How Akatsuki protects your mail.",
      body: (
        <dl className="spec">
          {MODEL.map(([k, v]) => (
            <div key={k} className="spec-row">
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      ),
    },
    ...doc.sections.map((s, i) => ({
      id: s.id,
      num: String(i + 2).padStart(2, "0"),
      title: s.title,
      html: s.html.replace(
        /<span class="hn">[^<]*<\/span>/,
        `<span class="hn">${String(i + 2).padStart(2, "0")}</span>`,
      ),
    })),
  ];
  return (
    <DocLayout
      current="/security"
      crumb="Security"
      title="Security"
      lead={
        <>
          Akatsuki handles other people’s email, so security reports come first. Found something? Write to{" "}
          <a className="tlink" href={mailto("SECURITY: ")} style={{ fontSize: 17 }}>
            {SITE.contact}
          </a>{" "}
          with a subject starting “SECURITY:”.
        </>
      }
      meta={`Updated 20 Oct 2026 · ${readMinutes(doc.words) + 1} min read · Source SECURITY.md`}
      parts={parts}
    />
  );
}
