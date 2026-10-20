import "../doc.css";
import type { Metadata } from "next";
import { PageHeader } from "@/components/PageHeader";
import { Shell } from "@/components/Shell";
import { SITE, WEB_LIVE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Download",
  description:
    "Akatsuki for the web, macOS, Linux, iPhone and Android is in development. Join the waitlist to hear when each one opens.",
  alternates: { canonical: "/download" },
};

const BLOCKS = [
  {
    kind: "Desktop",
    about:
      "Native apps written in Rust. A full local copy of your mail metadata, so lists open in under a millisecond and work offline.",
    rows: [
      {
        name: "macOS",
        desc: "Mail renders natively. Open original shows the sanitised message in a sandboxed window.",
      },
      { name: "Linux", desc: "Developed on Linux and Wayland, alongside macOS, not ported after it." },
    ],
  },
  {
    kind: "Web",
    about: "The whole client in a browser tab.",
    rows: [
      {
        name: "app.akatsuki.vc",
        desc: "Nothing to install. Message bodies are cached in your browser once you open them.",
        web: true,
      },
    ],
  },
  {
    kind: "Mobile",
    about: "Native apps for phones. Subscriptions are managed on the web.",
    rows: [
      { name: "iPhone", desc: "Designed around swipes: left to archive, right to snooze, tap to open." },
      { name: "Android", desc: "The same app and the same gestures, from one codebase shared with iPhone." },
    ],
  },
];

export default function DownloadPage() {
  return (
    <Shell current="/download">
      <div className="container doc">
        <PageHeader
          crumb="Download"
          title="Coming to every screen"
          lead="Every app is in development. Join the waitlist to hear when each one opens."
          meta="Status for every app: in development"
        />
        <div className="dl">
          {BLOCKS.map((b) => (
            <section key={b.kind} className="dl-block" aria-labelledby={`dl-${b.kind}`}>
              <div className="dl-l">
                <h2 className="dl-kind" id={`dl-${b.kind}`}>
                  {b.kind}
                </h2>
                <p className="dl-about">{b.about}</p>
              </div>
              <ul className="dl-rows">
                {b.rows.map((r) => (
                  <li key={r.name} className="dl-row">
                    <span className="dl-name">{r.name}</span>
                    <span className="dl-desc">{r.desc}</span>
                    {"web" in r && WEB_LIVE && (
                      <a className="btn btn-ghost dl-open" href={SITE.appUrl}>
                        Open
                      </a>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
        <div className="dl-after">
          <a className="btn btn-primary btn-block-m" href="/#waitlist">
            Join the waitlist
          </a>
          <p className="small">
            Rather run it yourself? Self-hosting is free for personal,{" "}
            <span className="nowrap">non-commercial</span> use.{" "}
            <a className="tlink" href="/license" style={{ fontSize: 14 }}>
              How self-hosting works <span className="arr">→</span>
            </a>
          </p>
        </div>
      </div>
    </Shell>
  );
}
