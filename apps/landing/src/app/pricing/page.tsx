import "./pricing.css";
import type { Metadata } from "next";
import { Hy, Nb } from "@/components/Nb";
import { PageHeader } from "@/components/PageHeader";
import { Shell } from "@/components/Shell";
import { BILLING_LIVE, mailto, SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Pricing",
  description:
    "Akatsuki Pro is $20 a month with a 14-day trial: hosted sync, every app and managed AI. Self-hosting is free for personal, non-commercial use.",
  alternates: { canonical: "/pricing" },
};

// The plan as one card: what is included on the right, as a plain list, then the total.
const ITEMS: [string, string][] = [
  ["Gmail, Outlook and any IMAP account", "Included"],
  ["Web, desktop and mobile apps", "Included"],
  ["Hosted sync for every account", "Included"],
  ["Splits, snooze, reminders and send later", "Included"],
  ["Undo send and search with operators", "Included"],
  ["Managed AI", "Monthly allowance"],
  ["Your own key or a local model", "Any time"],
  ["Message bodies stored on our servers", "Never"],
];

const COMPARE = [
  ["Price", "Free", "$20 a month"],
  ["Who runs the servers", "You", "Akatsuki"],
  ["Mail providers", "Gmail, Outlook, IMAP", "Gmail, Outlook, IMAP"],
  ["AI", "Your provider, or a local model", "Managed, or your own"],
  ["Google and Microsoft sign-in", "Bring your own OAuth clients", "Included"],
];

const FAQ = [
  ["Is there a free hosted plan?", "No. Self-hosting is free for personal, non-commercial use."],
  [
    "Do I need a card for the trial?",
    "Yes. The trial is 14 days, and the subscription starts when it ends unless you cancel.",
  ],
  ["What happens when the AI allowance runs out?", "You can switch to your own key or a local model."],
  [
    "Can I use my own Anthropic or OpenAI key on Pro?",
    "Yes, per feature. Your keys stay on the device you typed them into.",
  ],
  [
    "Do you store my email?",
    "Headers and a 200-character snippet, so lists and search work. Never message bodies or attachments.",
  ],
  [
    "Is Akatsuki open source?",
    "Yes, and it’s free for personal use. Commercial use, including by any organisation, needs permission. The licence is ASAL-1.0.",
  ],
  [
    "Can I use it at work?",
    "Business use needs a commercial licence, self-hosted or not. Write to us and we’ll sort one out.",
  ],
];

export default function PricingPage() {
  return (
    <Shell current="/pricing">
      <div className="container pr">
        <PageHeader
          crumb="Pricing"
          title="One plan for the hosted app"
          lead={
            <>
              Self-hosting stays free for personal, <span className="nowrap">non-commercial</span> use.
            </>
          }
          meta="Prices in US dollars · Pre-alpha"
        />

        <section className="pro" aria-labelledby="pro-title">
          <span className="pro-rim" aria-hidden="true" />
          <div className="grid pro-in">
            <div className="pro-l">
              <h2 id="pro-title" className="pro-name">
                Akatsuki Pro
              </h2>
              <p className="pro-price">
                <span className="price pr-num">$20</span>
                <span className="pro-per">a month</span>
              </p>
              <p className="pro-terms">
                14 days free. A card is needed to start the trial, and the subscription begins when it ends.
              </p>
              <div className="pro-actions">
                <a
                  className="btn btn-primary btn-block-m"
                  href={BILLING_LIVE ? `${SITE.appUrl}/signup` : "/#waitlist"}
                >
                  {BILLING_LIVE ? "Start your 14-day trial" : "Join the waitlist"}
                </a>
                <a className="tlink" href="/license">
                  Self-host instead <span className="arr">→</span>
                </a>
              </div>
            </div>
            <div className="pro-r">
              <ul className="pro-list" aria-label="What Pro includes">
                {ITEMS.map(([item, v]) => (
                  <li key={item}>
                    <span className="pro-item">{item}</span>
                    <span className="pro-v">{v}</span>
                  </li>
                ))}
              </ul>
              <dl className="pro-sum">
                <div>
                  <dt>Days 1 to 14</dt>
                  <dd>$0</dd>
                </div>
                <div className="is-total">
                  <dt>Each month after</dt>
                  <dd>$20</dd>
                </div>
              </dl>
            </div>
          </div>
          <p className="pro-foot">Nothing is billed until Pro launches.</p>
        </section>

        <section className="pr-sec" aria-labelledby="self-title">
          <div className="grid pr-self">
            <h2 id="self-title" className="h2 pr-h2">
              <span className="ln">Self-hosted is free</span>{" "}
              <span className="ln tone">
                for personal,{" "}
                <span className="nowrap">
                  <Hy>non-commercial</Hy> use
                </span>
              </span>
            </h2>
            <p className="body pr-self-body">
              You run the servers, bring your own Google and Microsoft OAuth clients, and pick your own AI
              provider or a local model. It is licensed under ASAL-1.0: open source, free for personal use.
            </p>
          </div>
          <div className="cmp-wrap">
            <table className="cmp">
              <thead>
                <tr>
                  <th scope="col">
                    <span className="sr-only">Feature</span>
                  </th>
                  <th scope="col">Self-hosted</th>
                  <th scope="col" className="is-pro">
                    Pro
                  </th>
                </tr>
              </thead>
              <tbody>
                {COMPARE.map(([k, a, b]) => (
                  <tr key={k}>
                    <th scope="row">
                      <Nb>{k ?? ""}</Nb>
                    </th>
                    <td>{a}</td>
                    <td>{b}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="pr-foot">
            Using Akatsuki for a business, self-hosted or not, needs a commercial licence.{" "}
            <a className="pr-mail" href={mailto("Akatsuki commercial licence")}>
              {SITE.contact}
            </a>
          </p>
        </section>

        <section className="pr-sec" aria-labelledby="faq-title">
          <h2 id="faq-title" className="h2 faq-h">
            Questions
          </h2>
          <dl className="faq">
            {FAQ.map(([q, a]) => (
              <div key={q} className="faq-item">
                <dt>{q}</dt>
                <dd>
                  <Nb>{a ?? ""}</Nb>
                  {q === "Can I use it at work?" && (
                    <>
                      {" "}
                      <a className="pr-mail" href={mailto("Akatsuki commercial licence")}>
                        {SITE.contact}
                      </a>
                    </>
                  )}
                </dd>
              </div>
            ))}
          </dl>
        </section>
      </div>
    </Shell>
  );
}
