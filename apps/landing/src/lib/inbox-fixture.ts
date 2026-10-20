// The only place the demo inbox's copy lives. Every person and company here is fictional.

export type Split = "Important" | "Calendar" | "Newsletters" | "Other";
export const SPLITS: readonly Split[] = ["Important", "Calendar", "Newsletters", "Other"];

export interface Thread {
  id: string;
  split: Split;
  from: string;
  /** Sender address, used by the from: search operator. */
  addr: string;
  subject: string;
  snippet: string;
  time: string;
  unread: boolean;
  starred: boolean;
  clip: boolean;
  chip?: string;
  summary: [string, string];
  body: [string, string, string];
}

export const THREADS: readonly Thread[] = [
  {
    id: "t1",
    split: "Important",
    from: "Hana Mori",
    addr: "hana@mori.studio",
    subject: "Offsite agenda for Thursday",
    snippet: "Draft attached. Could you take the 10:30 slot?",
    time: "09:41",
    unread: true,
    starred: false,
    clip: true,
    summary: [
      "Hana wants you to take the 10:30 roadmap slot on Thursday.",
      "The agenda draft is attached; she needs an answer by Wednesday.",
    ],
    body: [
      "Hi, attaching the draft agenda for Thursday.",
      "Lukas has the morning block, so could you take 10:30 for the roadmap review?",
      "Let me know by Wednesday and I'll send it round.",
    ],
  },
  {
    id: "t2",
    split: "Important",
    from: "Lukas Brenner",
    addr: "lukas@brenner.design",
    subject: "Re: Design review notes",
    snippet: "Agree on the spacing. One question on the empty state.",
    time: "09:12",
    unread: true,
    starred: false,
    clip: false,
    summary: [
      "Lukas agrees with the spacing changes from the review.",
      "He asks whether the empty state should show a hint or stay blank.",
    ],
    body: [
      "Agree on the spacing, the denser list reads better.",
      "One question on the empty state: do we show the shortcut hint, or leave it blank?",
      "I lean towards the hint for the first week only.",
    ],
  },
  {
    id: "t3",
    split: "Important",
    from: "Ren Takahashi",
    addr: "ren@takahashi.jp",
    subject: "Flight moved to 18:20",
    snippet: "Same gate, new boarding time. Dinner still on?",
    time: "08:57",
    unread: false,
    starred: true,
    clip: false,
    chip: "Travel",
    summary: [
      "Ren's flight now leaves at 18:20 from the same gate.",
      "He wants to know if dinner is still on.",
    ],
    body: [
      "The airline moved us to 18:20. Same gate, boarding at 17:50.",
      "That still leaves time for an early dinner near the station.",
      "Dinner still on? I can book for 16:00.",
    ],
  },
  {
    id: "t4",
    split: "Important",
    from: "Amara Okafor",
    addr: "amara@okafor.law",
    subject: "Quick question about the contract",
    snippet: "Section 4 has a renewal date that doesn't match the order form.",
    time: "08:30",
    unread: true,
    starred: false,
    clip: false,
    summary: [
      "Section 4 says the contract renews on 1 March; the order form says 1 April.",
      "Amara wants to know which date is right before she signs on Friday.",
    ],
    body: [
      "Section 4 has a renewal date that doesn't match the order form.",
      "The contract says 1 March, the order form says 1 April.",
      "Which one is right? I'd like to sign on Friday.",
    ],
  },
  {
    id: "t10",
    split: "Important",
    from: "Mira Lindqvist",
    addr: "mira@lindqvist.se",
    subject: "Q4 budget, final numbers",
    snippet: "Two lines changed since Friday. Can you sign off by noon?",
    time: "08:14",
    unread: true,
    starred: false,
    clip: true,
    summary: [
      "Two budget lines changed since Friday: travel went down, tooling went up.",
      "Mira needs your sign-off by noon today.",
    ],
    body: [
      "Final numbers for Q4 are attached.",
      "Two lines changed since Friday: travel is down a little, tooling is up.",
      "Can you sign off by noon so I can send it on?",
    ],
  },
  {
    id: "t11",
    split: "Important",
    from: "Tomás Ferreira",
    addr: "tomas@ferreira.pt",
    subject: "Re: Onboarding copy",
    snippet: "Shortened the second screen. Have a look when you can.",
    time: "07:52",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "Tomás cut the second onboarding screen to two sentences.",
      "He wants a quick look before it goes to translation.",
    ],
    body: [
      "Shortened the second screen to two sentences.",
      "The first and third stay as they were.",
      "Have a look when you can; translation starts Thursday.",
    ],
  },
  {
    id: "t12",
    split: "Important",
    from: "Kiln Works IT",
    addr: "it@kiln.works",
    subject: "Your new laptop is ready",
    snippet: "Pick it up from desk 4 any time after 10.",
    time: "07:30",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "Your replacement laptop is set up and waiting at desk 4.",
      "Bring the old one; pickup is from 10:00.",
    ],
    body: [
      "Your new laptop is set up and ready.",
      "Pick it up from desk 4 any time after 10.",
      "Please bring the old one with you.",
    ],
  },
  {
    id: "t5",
    split: "Important",
    from: "Hiring team",
    addr: "hiring@kiln.works",
    subject: "Three candidates for Friday",
    snippet: "Profiles and the interview plan are in the doc.",
    time: "Yesterday",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "Three candidates are booked for Friday afternoon.",
      "Profiles and the interview plan are in the shared doc.",
    ],
    body: [
      "Three candidates are confirmed for Friday, from 13:00.",
      "Profiles and the interview plan are in the doc.",
      "Please add your questions by Thursday evening.",
    ],
  },
  {
    id: "t13",
    split: "Important",
    from: "Jonas Weber",
    addr: "jonas@weber-legal.de",
    subject: "Contract countersigned",
    snippet: "Attached for your records. Thanks for the quick turnaround.",
    time: "Yesterday",
    unread: false,
    starred: false,
    clip: true,
    summary: ["The supplier contract is countersigned and attached.", "Nothing else is needed from you."],
    body: [
      "The countersigned contract is attached for your records.",
      "Thanks for the quick turnaround on the last round of edits.",
      "Nothing else is needed from your side.",
    ],
  },
  {
    id: "t14",
    split: "Important",
    from: "Elif Aydın",
    addr: "elif@aydin.studio",
    subject: "Photos from the offsite",
    snippet: "The good ones are starred in the shared album.",
    time: "Yesterday",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "Elif uploaded the offsite photos to the shared album.",
      "The best ones are starred, in case you need some for the newsletter.",
    ],
    body: [
      "All the photos from the offsite are in the shared album.",
      "The good ones are starred.",
      "Use any of them for the newsletter.",
    ],
  },
  {
    id: "t15",
    split: "Important",
    from: "Priya Raman",
    addr: "priya@raman.io",
    subject: "Lunch on Wednesday?",
    snippet: "The new place by the river opens at 12.",
    time: "Mon",
    unread: true,
    starred: false,
    clip: false,
    summary: [
      "Priya suggests lunch on Wednesday at the new place by the river.",
      "It opens at 12:00; she can book if you say yes.",
    ],
    body: [
      "Lunch on Wednesday?",
      "The new place by the river opens at 12.",
      "I can book a table if you're in.",
    ],
  },
  {
    id: "t16",
    split: "Important",
    from: "Noah Fischer",
    addr: "noah@fischer.works",
    subject: "Re: Hiring plan for Q1",
    snippet: "Two roles, not three. Reasons inline.",
    time: "Mon",
    unread: false,
    starred: true,
    clip: false,
    summary: [
      "Noah thinks Q1 needs two hires, not three.",
      "His reasons are inline; he wants to settle it this week.",
    ],
    body: [
      "Two roles, not three. Reasons inline.",
      "The third can wait until the search work is scoped.",
      "Can we settle it this week?",
    ],
  },
  {
    id: "t6",
    split: "Calendar",
    from: "Calendar",
    addr: "calendar@kiln.works",
    subject: "Invitation: Design review, Thu 14:00",
    snippet: "Lukas Brenner invited you · Room 3",
    time: "Yesterday",
    unread: true,
    starred: false,
    clip: false,
    summary: [
      "Lukas invited you to the design review on Thursday at 14:00.",
      "It is in Room 3 and runs for an hour.",
    ],
    body: [
      "Design review. Thursday, 14:00 to 15:00.",
      "Room 3. Organiser: Lukas Brenner.",
      "Accept, decline or propose a new time.",
    ],
  },
  {
    id: "t7",
    split: "Newsletters",
    from: "The Weekly Margin",
    addr: "issue@weeklymargin.news",
    subject: "Issue 112: rebuilding search",
    snippet: "What we learned moving to a local index.",
    time: "Mon",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "This issue is about moving search to a local index.",
      "Queries got faster; keeping the index fresh got harder.",
    ],
    body: [
      "This week: what we learned moving search to a local index.",
      "Queries got faster. Keeping the index fresh got harder.",
      "Plus three links worth your time.",
    ],
  },
  {
    id: "t8",
    split: "Newsletters",
    from: "Field Notes",
    addr: "hello@fieldnotes.letter",
    subject: "Five tools we cut this year",
    snippet: "And the one we brought back.",
    time: "Sun",
    unread: false,
    starred: false,
    clip: false,
    summary: [
      "The team dropped five tools this year to simplify their week.",
      "One came back after a month without it.",
    ],
    body: [
      "We cut five tools this year.",
      "Fewer tabs, fewer logins, fewer notifications.",
      "And the one we brought back after a month without it.",
    ],
  },
  {
    id: "t9",
    split: "Other",
    from: "Northwind Billing",
    addr: "billing@northwind.example",
    subject: "Invoice INV-2041 for October",
    snippet: "€48.00, paid. The PDF is attached.",
    time: "Oct 1",
    unread: false,
    starred: false,
    clip: true,
    chip: "Receipts",
    summary: ["Invoice INV-2041 for October is paid: €48.00.", "The PDF is attached. Nothing to do."],
    body: [
      "Thanks for your payment.",
      "Invoice INV-2041 for October, €48.00, paid on 1 October.",
      "The PDF is attached for your records.",
    ],
  },
];

/** The visitor's three accounts in the demo: one per provider kind, each with its own monogram. */
export const ACCOUNTS = [
  { letter: "K", addr: "sam@kiln.works", kind: "Outlook" },
  { letter: "S", addr: "sam@samito.studio", kind: "Gmail" },
  { letter: "I", addr: "sam@ito.family", kind: "IMAP" },
] as const;

export const LABELS = [
  { name: "Clients", count: 4 },
  { name: "Travel", count: 2 },
  { name: "Receipts", count: 9 },
] as const;

/** Sidebar views and their G-key. Counts other than Inbox and Snoozed are static. */
export const VIEWS = [
  { id: "inbox", label: "Inbox", key: "I" },
  { id: "starred", label: "Starred", key: "S" },
  { id: "snoozed", label: "Snoozed", key: "H" },
  { id: "reminders", label: "Reminders", key: "R" },
  { id: "sent", label: "Sent", key: "T" },
  { id: "drafts", label: "Drafts", key: "D" },
] as const;
export type ViewId = (typeof VIEWS)[number]["id"];

export const EMPTY_VIEWS: Record<string, string> = {
  starred: "Nothing starred yet.",
  reminders: "No reminders set.",
  sent: "Nothing sent from the demo.",
  drafts: "One draft, safely elsewhere.",
  snoozed: "Nothing snoozed yet.",
};

export const SNOOZE_OPTIONS = [
  { key: "1", label: "Later today", when: "18:00" },
  { key: "2", label: "Tomorrow", when: "08:00" },
  { key: "3", label: "This weekend", when: "Sat 08:00" },
  { key: "4", label: "Next week", when: "Mon 08:00" },
  { key: "P", label: "Pick a date…", when: "" },
] as const;
