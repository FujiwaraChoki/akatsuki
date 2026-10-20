import "../doc.css";
import type { Metadata } from "next";
import { PageHeader } from "@/components/PageHeader";
import { Shell } from "@/components/Shell";

export const metadata: Metadata = {
  title: "Changelog",
  description: "What changed in Akatsuki, release by release.",
  alternates: { canonical: "/changelog" },
};

const ENTRIES = [
  {
    date: "2026-10-20",
    label: "20 Oct 2026",
    title: "Pre-alpha: the foundations",
    items: [
      "Architecture decided: Convex for metadata and live sync, a Rust sync worker that is the only holder of mailbox credentials, and message bodies that never touch our database.",
      "Shared design tokens and one key map for every app.",
      "AI layer with bring-your-own-model support, including local Ollama, and a prompt pack shared by the TypeScript and Rust clients.",
      "Backend, sync worker, API service and desktop shell under construction. Nothing is ready to use yet.",
    ],
  },
];

export default function ChangelogPage() {
  return (
    <Shell current="/changelog">
      <div className="container doc">
        <PageHeader
          crumb="Changelog"
          title="Changelog"
          lead="There is no public release yet. Every release will be listed here, newest first."
          meta="1 entry · Pre-alpha"
        />
        <div className="doc-body" style={{ display: "block" }}>
          {ENTRIES.map((e) => (
            <article key={e.date} className="cl-entry">
              <time className="cl-date" dateTime={e.date}>
                {e.label}
              </time>
              <div className="cl-body prose">
                <h2 className="cl-title">{e.title.replace(/\.$/, "")}</h2>
                <ul>
                  {e.items.map((i) => (
                    <li key={i}>{i}</li>
                  ))}
                </ul>
              </div>
            </article>
          ))}
        </div>
      </div>
    </Shell>
  );
}
