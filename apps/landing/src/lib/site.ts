// Site-wide facts. Pages, nav, sitemap, the palette and the footer read from here.
// Snapshot of apps/landing/src/lib/site.ts (facts only).

export const SITE = {
  name: "Akatsuki",
  url: "https://akatsuki.vc",
  appUrl: "https://app.akatsuki.vc",
  contact: "sami@samihindi.com",
  owner: "Sami Hindi",
  title: "Akatsuki: the open-source Superhuman alternative",
  tagline: "Clear your inbox by first light.",
  description:
    "Akatsuki is the open-source Superhuman alternative: an email client you drive from the keyboard. Gmail, Outlook and any IMAP account in one inbox, on the web, your desktop and your phone, with AI you can run on any provider, including a local model. Open source, free for personal use.",
} as const;

/** The waitlist note, previewed on the page and opened in the visitor's mail app. */
export const WL_SUBJECT = "Akatsuki waitlist";
export const WL_LINE = "Please add me to the Akatsuki waitlist.";

/** Flips the pricing call to action once checkout exists. */
export const BILLING_LIVE = false;
/** Shows an "Open" button for the web app on /download once it is public. */
export const WEB_LIVE = false;

export interface PageInfo {
  href: string;
  title: string;
  summary: string;
  nav: boolean;
  changeFrequency: "weekly" | "monthly" | "yearly";
  priority: number;
}

export const PAGES: readonly PageInfo[] = [
  {
    href: "/",
    title: "Home",
    summary: "What Akatsuki is",
    nav: false,
    changeFrequency: "weekly",
    priority: 1,
  },
  {
    href: "/pricing",
    title: "Pricing",
    summary: "Pro at $20 a month, or self-host for free",
    nav: true,
    changeFrequency: "monthly",
    priority: 0.9,
  },
  {
    href: "/download",
    title: "Download",
    summary: "Web, desktop and mobile, all in development",
    nav: true,
    changeFrequency: "weekly",
    priority: 0.8,
  },
  {
    href: "/changelog",
    title: "Changelog",
    summary: "What changed, release by release",
    nav: true,
    changeFrequency: "weekly",
    priority: 0.5,
  },
  {
    href: "/privacy",
    title: "Privacy",
    summary: "What we store, where, and for how long",
    nav: true,
    changeFrequency: "monthly",
    priority: 0.5,
  },
  {
    href: "/security",
    title: "Security",
    summary: "How your mail is protected, and how to report a problem",
    nav: false,
    changeFrequency: "monthly",
    priority: 0.6,
  },
  {
    href: "/license",
    title: "Licence",
    summary: "Open source, free for personal use",
    nav: false,
    changeFrequency: "yearly",
    priority: 0.5,
  },
  {
    href: "/terms",
    title: "Terms",
    summary: "Terms of service (draft)",
    nav: false,
    changeFrequency: "yearly",
    priority: 0.3,
  },
];

export const mailto = (subject: string, body = ""): string =>
  `mailto:${SITE.contact}?subject=${encodeURIComponent(subject)}${body ? `&body=${encodeURIComponent(body)}` : ""}`;
