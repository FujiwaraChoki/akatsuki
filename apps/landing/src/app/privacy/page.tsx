import "../doc.css";
import type { Metadata } from "next";
import { DocLayout, type DocPart } from "@/components/DocLayout";
import { SpecSheet } from "@/components/SpecSheet";
import { readContent, readMinutes, renderDoc } from "@/lib/markdown";

export const metadata: Metadata = {
  title: "Privacy",
  description:
    "What Akatsuki stores, where, and for how long. Message bodies and attachments are never stored on our servers. Draft pending lawyer review.",
  alternates: { canonical: "/privacy" },
};

const NOTES: Record<string, string> = {
  "who-is-responsible": "One person runs Akatsuki and answers every data request.",
  "what-akatsuki-stores-and-where":
    "Headers and short snippets in the database. Message bodies only on your own devices.",
  "ai-features":
    "A local model keeps everything on your machine. Managed dictation is the one exception to zero retention.",
  "the-website": "No analytics and no cookies. Your theme choice stays in your browser.",
  "how-long-we-keep-data":
    "Thirty days after a subscription ends, mail metadata and credentials are deleted.",
  "your-rights": "See, export, correct or delete your data. Write, and you get an answer within 30 days.",
};

export default function PrivacyPage() {
  const doc = renderDoc(readContent("privacy"));
  const parts: DocPart[] = doc.sections.map((s) =>
    s.id === "the-short-version"
      ? {
          id: s.id,
          num: s.num,
          title: "The short version.",
          note: "This table is the summary. Everything below it is the full policy.",
          body: (
            <>
              <SpecSheet />
              {/* biome-ignore lint/security/noDangerouslySetInnerHtml: our own checked-in documentation, rendered at build time */}
              <div dangerouslySetInnerHTML={{ __html: s.html.replace(/<h2[\s\S]*?<\/h2>/, "") }} />
            </>
          ),
        }
      : { id: s.id, num: s.num, title: s.title, html: s.html, note: NOTES[s.id] },
  );
  return (
    <DocLayout
      current="/privacy"
      crumb="Privacy"
      title="Privacy"
      meta={`Updated 20 Oct 2026 · ${readMinutes(doc.words)} min read · Draft, pending legal review`}
      intro={
        // biome-ignore lint/security/noDangerouslySetInnerHtml: our own checked-in documentation, rendered at build time
        <div dangerouslySetInnerHTML={{ __html: doc.intro }} />
      }
      parts={parts}
    />
  );
}
