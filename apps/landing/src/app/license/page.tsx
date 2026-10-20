import "../doc.css";
import type { Metadata } from "next";
import { DocLayout, type DocPart } from "@/components/DocLayout";
import { readContent } from "@/lib/markdown";
import { mailto, SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Licence",
  description:
    "Akatsuki is open source, free for personal use, under the Akatsuki Source-Available License (ASAL-1.0). Commercial use, including by any organisation, needs written permission.",
  alternates: { canonical: "/license" },
};

const MAY = [
  "Read and study the source code",
  "Run Akatsuki for yourself, including self-hosting it",
  "Let your household and friends use your self-hosted copy, as long as nobody pays",
  "Change the code for your own use",
  "Publish your changes as source, free, under the same licence",
  "Contribute changes back",
];
const MAY_NOT = [
  "Use Akatsuki in or for a business, or for any commercial purpose",
  "Sell Akatsuki, or sell access to it",
  "Offer it to others as a hosted or managed service",
  "Bundle it, or any part of it, into a paid product",
  "Use the Akatsuki name or logo for your copy",
];
const ASK = [
  "Use by an organisation of any kind, including non-profits, schools and public bodies",
  "Using a self-hosted copy for business email, including your own freelance or company mail",
  "Publishing compiled builds in an app store or package registry",
];

const List = ({ items }: { items: string[] }) => (
  <ul>
    {items.map((x) => (
      <li key={x}>{x}</li>
    ))}
  </ul>
);

export default function LicensePage() {
  const text = readContent("licence");
  const parts: DocPart[] = [
    {
      id: "in-brief",
      num: "01",
      title: "In brief.",
      note: "Personal and non-commercial: yes. Anything commercial: ask first.",
      body: (
        <div className="lic-cols">
          <div>
            <span className="lic-h">You may</span>
            <List items={MAY} />
          </div>
          <div>
            <span className="lic-h">You may not</span>
            <List items={MAY_NOT} />
          </div>
          <div>
            <span className="lic-h">Ask first</span>
            <List items={ASK} />
          </div>
        </div>
      ),
    },
    {
      id: "personal-use",
      num: "02",
      title: "What “free for personal use” means.",
      note: "The code is open to everyone. The commercial rights stay with its maker.",
      body: (
        <p>
          You can use Akatsuki for yourself, and share a self-hosted copy with your household and friends, for
          free, as long as nobody pays. Anything commercial is reserved: using it in or for a business, use by
          any organisation (including non-profits, schools and public bodies), selling it or offering it as a
          hosted service needs written permission from the copyright holder first. That reservation lets the
          hosted service pay for Akatsuki’s development, while the code stays open for anyone who wants to
          check what an email client does with their mail. Because it reserves commercial use, ASAL-1.0 is not
          an OSI-approved licence.
        </p>
      ),
    },
    {
      id: "self-hosting",
      num: "03",
      title: "Self-hosting.",
      body: (
        <p>
          Self-hosting is free for personal, non-commercial use. You bring your own Google and Microsoft OAuth
          clients and your own AI provider, or run a model locally. Setup instructions will be published with
          the source. Using Akatsuki for a business, self-hosted or not, needs a commercial licence: write to{" "}
          <a href={mailto("Akatsuki commercial licence")}>{SITE.contact}</a>.
        </p>
      ),
    },
    {
      id: "contributions",
      num: "04",
      title: "Contributions.",
      body: (
        <p>
          You keep the copyright in what you contribute. By contributing you grant the copyright holder a
          perpetual, irrevocable licence to use and relicense your contribution, including commercially. The
          details are in section 6 of the licence.
        </p>
      ),
    },
    {
      id: "third-party",
      num: "05",
      title: "Third-party components.",
      body: (
        <p>
          Libraries Akatsuki uses keep their own licences. The Convex backend is licensed under
          FSL-1.1-Apache-2.0; if you run it yourself you follow that licence too. The hosted service, Akatsuki
          Pro, is covered by its own <a href="/terms">terms of service</a>, not by this licence.
        </p>
      ),
    },
    {
      id: "full-text",
      num: "06",
      title: "The full licence.",
      note: "This text has not yet been reviewed by a lawyer. It will be before any commercial launch.",
      body: (
        <>
          <p>This summary is for convenience. The licence text below is what applies.</p>
          <pre className="legal-text">{text}</pre>
        </>
      ),
    },
  ];
  return (
    <DocLayout
      current="/license"
      crumb="Licence"
      title="Licence"
      lead="Open source, free for personal use. You can read, run and change Akatsuki for personal, non-commercial use. Commercial use of any kind, including by any organisation, needs written permission first."
      meta="ASAL-1.0 · Version 1.0 · Source LICENSE"
      parts={parts}
    />
  );
}
