import "../doc.css";
import type { Metadata } from "next";
import { DocLayout, type DocPart } from "@/components/DocLayout";
import { SITE } from "@/lib/site";

export const metadata: Metadata = {
  title: "Terms",
  description: "Terms of service for Akatsuki Pro. Draft pending lawyer review; not yet in force.",
  alternates: { canonical: "/terms" },
};

const P = ({ children }: { children: React.ReactNode }) => <p>{children}</p>;

const PARTS: DocPart[] = [
  {
    id: "who",
    num: "01",
    title: "Who we are.",
    body: (
      <P>
        Akatsuki is provided by {SITE.owner} (“we”, “us”). You can reach us at{" "}
        <a href={`mailto:${SITE.contact}`}>{SITE.contact}</a>. These terms cover the hosted service at
        akatsuki.vc and its apps (“Akatsuki Pro” or “the service”). Running the software yourself is covered
        by the <a href="/license">licence</a>, not by these terms.
      </P>
    ),
  },
  {
    id: "account",
    num: "02",
    title: "Your account.",
    body: (
      <ul>
        <li>You need to be at least 16 years old and able to agree to these terms.</li>
        <li>
          Keep your sign-in methods secure. You are responsible for what happens in your account, and for
          having the right to connect each mailbox you add.
        </li>
        <li>Connected mailboxes stay yours. You can disconnect them, or delete your account, at any time.</li>
      </ul>
    ),
  },
  {
    id: "subscription",
    num: "03",
    title: "Subscription, trial and payment.",
    note: "$20 a month. One 14-day trial, which needs a card. Cancel any time.",
    body: (
      <ul>
        <li>Akatsuki Pro costs $20 per month, billed monthly in advance through Stripe. Taxes may apply.</li>
        <li>
          New customers get one 14-day free trial. Starting it needs a payment method. If you do not cancel
          before the trial ends, your subscription starts and your card is charged.
        </li>
        <li>
          You can cancel at any time. Cancelling stops renewal; the service keeps working until the end of the
          period you have paid for. We do not refund partial months unless the law requires it.
        </li>
        <li>
          If a payment fails, we keep the service running for a short grace period while Stripe retries, and
          tell you by email.
        </li>
        <li>
          We will give you at least 30 days’ notice of a price change, and it applies from your next period.
        </li>
      </ul>
    ),
  },
  {
    id: "ai",
    num: "04",
    title: "Managed AI.",
    body: (
      <P>
        Pro includes managed AI features with a monthly usage allowance, measured at the list price of the
        models used, and rate limits that keep the service fair for everyone. When the allowance runs out,
        managed AI pauses until the next month; you can switch to your own provider key or a local model at
        any time, which Akatsuki does not meter. AI output can be wrong. Check it before you rely on it or
        send it.
      </P>
    ),
  },
  {
    id: "your-content",
    num: "05",
    title: "Your content.",
    note: "Your mail is yours. It is never sold, used for ads or used to train models.",
    body: (
      <P>
        Your email and everything in it belongs to you. You give us permission to process it only as needed to
        run the service for you: to sync, display, search, send and, when you ask, run AI features. We do not
        sell it, use it for advertising, or use it to train models. The <a href="/privacy">privacy draft</a>{" "}
        explains what we store and for how long.
      </P>
    ),
  },
  {
    id: "acceptable-use",
    num: "06",
    title: "Acceptable use.",
    body: (
      <>
        <P>Do not use Akatsuki to:</P>
        <ul>
          <li>send spam or bulk unsolicited email, or mail that breaks the law;</li>
          <li>access mailboxes you are not authorised to use;</li>
          <li>
            break, overload, probe or reverse-engineer the hosted service, except as our security policy
            allows;
          </li>
          <li>break the terms of your mail provider (Google, Microsoft or your IMAP host).</li>
        </ul>
        <P>
          We may suspend accounts that put the service or other people at risk, and will tell you why unless
          the law prevents it.
        </P>
      </>
    ),
  },
  {
    id: "ending",
    num: "07",
    title: "When the service ends for you.",
    body: (
      <P>
        If your subscription ends or your account is suspended, syncing stops. Thirty days later we delete
        your mail metadata and mailbox credentials. Deleting your account removes your data sooner. Your mail
        itself stays with your provider.
      </P>
    ),
  },
  {
    id: "availability",
    num: "08",
    title: "Availability and changes.",
    body: (
      <P>
        We work hard to keep Akatsuki fast and available, but we do not promise it will be uninterrupted or
        error-free, and there is no service-level agreement. Akatsuki depends on mail providers and other
        services we do not control. We may change or remove features; for material changes that reduce what
        you pay for, we will tell you in advance.
      </P>
    ),
  },
  {
    id: "liability",
    num: "09",
    title: "Warranty and liability.",
    body: (
      <P>
        The service is provided “as is”. To the extent the law allows, we are not liable for indirect or
        consequential losses, or for lost data, email or profit, and our total liability is limited to what
        you paid us in the 12 months before the claim. Nothing in these terms limits liability that cannot be
        limited by law, or your statutory rights as a consumer.
      </P>
    ),
  },
  {
    id: "changes",
    num: "10",
    title: "Changes to these terms.",
    body: (
      <P>
        If we change these terms, we will post the new version here and, for material changes, tell you by
        email or in the app at least 30 days before they apply.
      </P>
    ),
  },
  {
    id: "law",
    num: "11",
    title: "Governing law.",
    body: (
      <P>
        [To be completed after lawyer review: governing law, jurisdiction and consumer dispute resolution.]
      </P>
    ),
  },
];

export default function TermsPage() {
  return (
    <DocLayout
      current="/terms"
      crumb="Terms"
      title="Terms"
      meta="Draft of 20 Oct 2026 · 4 min read · Not yet in force"
      intro={
        <p className="draft-note">
          <strong>Draft pending lawyer review.</strong> These terms are not in force. They describe how we
          intend the service to work and will change before launch. Akatsuki is pre-alpha and the hosted
          service is not open to the public yet.
        </p>
      }
      parts={PARTS}
    />
  );
}
