# Privacy at Akatsuki

> **Draft for lawyer review.** This text describes how Akatsuki is designed to handle your data. It has not
> yet been reviewed by a lawyer and is not yet a binding privacy policy. Akatsuki is pre-alpha: the hosted
> service is not open to the public yet. Before launch this draft will be reviewed, completed (for example
> with data locations, legal bases and retention periods for backups) and dated.

Last updated: 20 October 2026 (draft).

Akatsuki is an email client. Reading your email is the whole point of the product, so we owe you a plain
account of where your mail goes, who can see it and what we keep. This page describes the hosted service
(Akatsuki Pro) and the website `akatsuki.vc`. If you run Akatsuki yourself, see
[Self-hosted copies](#self-hosted-copies).

## The short version

- **Your message bodies and attachments are never stored on our servers.** Our database (Convex) holds
  headers and short snippets so lists and search work. The text of your messages is fetched from your mail
  provider when you open it, passes through our sync service in memory, and is cached only on your own
  devices.
- **No trackers.** The website and the apps contain no analytics, no advertising, no tracking pixels and no
  third-party scripts. We do not sell or share your data for advertising.
- **AI is optional and you choose where it runs.** With a local model (for example Ollama) nothing leaves your
  machine. With your own provider key, the thread you run a feature on goes straight from your device to that
  provider. With managed AI on Pro, requests pass through our stateless AI service, which stores and logs
  nothing. Managed text features use zero-data-retention routing. **Managed dictation is the exception**: the
  audio is sent to OpenAI, which may keep it for up to 30 days for abuse monitoring (details below).
- **We do not use your mail to train models**, and we do not let our AI providers do so.

## Who is responsible

Akatsuki is run by Sami Hindi (the "controller" in data-protection terms). Contact:
**sami@samihindi.com**. Use the same address for any request about your data.

## What Akatsuki stores, and where

Akatsuki has a few separate parts. Each one sees only what it needs.

### Convex (our database)

Convex is a hosted database run by Convex, Inc. It is the control plane: it keeps your mailbox organised and
in sync between your devices. It stores:

- **Your login identity**: email address, name, the sign-in method you used (magic link, one-time code, or
  Google, Microsoft or GitHub sign-in), and your sessions and devices (device name, platform, push token if
  you enabled notifications).
- **Mail metadata**: for each connected mailbox, its folders and labels, and for each thread and message:
  sender, recipients, subject, date, flags (read, starred and so on), labels, whether it has attachments,
  its size, mailing-list id, sender authentication results (SPF, DKIM, DMARC), and a **snippet of up to 200
  characters** of the message. A search index built from subjects,
  participants and the last few snippets lives here too. This is what lets lists open instantly and work the
  same on every device.
- **Your state and settings**: snoozes, reminders, scheduled sends, split inbox definitions, filter rules,
  snippets and signatures you wrote yourself, sender preferences, and app settings.
- **Draft metadata**: the subject and recipients of drafts and scheduled sends. The draft text lives in your
  mail provider's drafts folder and on your devices.
- **Mailbox credentials, encrypted**: the OAuth refresh token (Gmail, Outlook) or the IMAP password needed to
  sync your mailbox. They are encrypted with XChaCha20-Poly1305 before they reach Convex. The key exists only
  on our sync server, so Convex holds ciphertext only.
- **Billing state**: your Stripe customer id, subscription status and trial dates. No card details.
- **AI**: your AI settings (which provider and model for which feature, but **never your API keys**), your
  consent record, usage counts for managed AI, the category and "needs reply" flag that auto-triage assigns
  to a thread, and, only if you opt in to syncing it, a short description of your writing style.

Convex **never** stores message bodies, attachments, AI prompts or AI outputs (other than the triage label
and the opt-in style profile above).

### The sync service (our server)

The sync service is a Rust program we run on a server rented from Hetzner Online GmbH. It is the only part
of Akatsuki that talks to your mail provider and the only part that can decrypt your mailbox credentials.

- It fetches new mail and changes, and writes the metadata above to Convex.
- When you open a message, it fetches the body from your provider and streams it to your device. The body is
  held in memory only for that request and is never written to disk or to the database.
- It sanitises HTML email (removing scripts, forms and tracking pixels) and proxies remote images when you
  allow them, so senders see our server's address, not yours. Images are blocked by default. This holds in
  every app: "Show images" in the desktop app also loads them through this proxy, never from the sender.
- When you attach a file, it is kept in an encrypted temporary spool for 24 hours, so every save of the draft
  and the send can attach it again, then deleted.

### The API service (our server)

A small stateless service on the same server handles managed AI, managed dictation and Stripe webhooks. It
never stores or logs request or response content. It records only usage counts so it can enforce the monthly
AI allowance.

### Your devices

The apps cache what you have opened so mail is fast and works offline: message bodies, attachments you
viewed, AI results and drafts. On desktop this cache is a local database with a full-text index; on the web
it is your browser's storage; on mobile it is the app's storage. You can turn body caching off in settings.
Your own AI provider keys are stored only on the device you typed them into (in the OS keychain on desktop,
secure storage on mobile, and either memory or encrypted browser storage on the web).

### Logs

Our services log ids, counts, timings and error codes. They never log subjects, snippets, addresses, message
text, prompts, AI output, tokens or search queries.

## AI features

AI in Akatsuki is optional. No AI feature runs until you have given consent, and the consent screen names the
provider and what leaves your device. Every AI action shows one of three labels: "Stays on this device",
"Leaves this device" or "Sent to Akatsuki for processing, not stored".

Only what a feature needs is sent: the one thread you run it on, or, for auto-triage, up to 25 items made of
headers and snippets. Triage never sends message bodies.

| Where AI runs | What leaves your device | Who can see it |
|---|---|---|
| **Local model** (Ollama, LM Studio, a local Whisper server, any endpoint on your machine or network) | Nothing | Only you |
| **Your own provider key** (OpenRouter, Anthropic, OpenAI, Google, Vercel AI Gateway, any OpenAI-compatible endpoint) | The thread or text the feature runs on, sent directly from your device | That provider, under its terms with you. Akatsuki never sees the request or your key. |
| **Managed AI on Pro, text features** (summaries, replies, writing, rewrite, action items, search, triage) | The thread or text the feature runs on, sent to our API service | Our API service passes it to OpenRouter with zero data retention required and data collection denied, so it is routed only to model providers that do not keep or train on it. Nothing is stored or logged by Akatsuki. |
| **Managed dictation on Pro** | Your audio recording, sent to our API service | Our API service passes it to **OpenAI's transcription API under OpenAI's standard API data policy**. OpenAI does not train on API data, but **may retain inputs for up to 30 days** for abuse monitoring. Akatsuki has not confirmed zero data retention with OpenAI. To keep audio on your machine, choose a local Whisper endpoint for dictation. |

AI results stay on your device. The only AI outputs that reach our database are the auto-triage category and
"needs reply" flag, and your writing-style profile if you choose to sync it.

**Google and Microsoft user data.** Akatsuki's use of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements. We apply the same rules to data from Microsoft: mailbox data is used
only to provide the features you use, is never used for advertising, is never sold, is never used to train
AI models, and is never read by people except with your explicit permission, for security reasons, or where
the law requires it.

## Notifications

Push notifications reach your phone only after you allow them on that phone. You choose one of three levels;
the default is **Private**:

- **Off**: nothing is sent.
- **Private**: you get "New mail" or "3 new messages", with no sender and no subject.
- **Full**: the notification shows the **sender's name and the subject**. This text travels through Expo's
  push service (Expo, the mobile app platform) and Apple's or Google's push network to reach your phone.

Message bodies are never sent in notifications.

## Read receipts

Read receipts are off by default. If you turn them on, messages you send carry a tiny image hosted by our
sync service. When a recipient's mail app loads it, we record the time and whether it came through a proxy
(such as Apple Mail Privacy Protection, which makes receipts unreliable). We do not record the recipient's IP
address. The app warns you about this before you turn the feature on.

## Email from us

We send transactional email only: sign-in links and codes, trial and billing reminders, and notices about
your account (for example if a mailbox needs to be reconnected or your data is about to be deleted). It is
sent through **Amazon SES** from `no-reply@akatsuki.vc`. We do not send marketing email, and the emails contain
no tracking pixels.

## Payments

Akatsuki Pro is billed through **Stripe**. Stripe collects and stores your card details and billing address
under its own privacy policy; we never see your full card number. We keep your Stripe customer id and
subscription status so we know whether you are on a trial or a paid plan.

## The website

`akatsuki.vc` is a static website hosted on **Vercel**. It has no analytics, no cookies and no third-party
scripts or fonts. Vercel processes standard request data (such as IP address and user agent) to serve the
site and protect it from abuse. Your theme choice (light, dark or system) is kept in your browser's local
storage and never sent anywhere. The waitlist form on the download page does not send anything to a server;
it opens an email to us in your own mail app.

## Who we share data with (sub-processors)

We share data only with the services that run Akatsuki, and only what each one needs:

| Service | What for | What it receives |
|---|---|---|
| Convex, Inc. | Database and backend functions | The data listed under "Convex" above |
| Hetzner Online GmbH | Server hosting for the sync and API services | Data in transit through those services; encrypted temporary attachment spool |
| Vercel Inc. | Hosting the website and the web app | Standard request data |
| Stripe | Payments | Billing details you give Stripe; your email address |
| Amazon Web Services (SES) | Transactional email | Your email address and the email content |
| OpenRouter, and the model providers it routes to with zero data retention | Managed AI text features (Pro) | The text a feature needs, per request, not retained |
| OpenAI | Managed dictation (Pro) | Your audio, retained for up to 30 days under OpenAI's API data policy |
| Expo, Apple, Google | Mobile push notifications | A device token and, at the "Full" level only, sender and subject |
| Cloudflare | DNS for `akatsuki.vc` | DNS queries, no application data |
| Google, Microsoft, your IMAP host | Your mailbox | Requests from our sync service on your behalf |

We do not sell personal data, we do not share it for advertising, and we do not use it to profile you.

## How long we keep data

- **While your account is active**, mail metadata is kept in sync with your mailbox: when a message is deleted
  at your provider, it is deleted from Akatsuki too.
- **Disconnecting a mailbox** deletes its metadata and credentials.
- **Deleting your account** removes all your mail metadata, credentials, settings and AI data, cancels your
  subscription, and deletes your login and its sessions. If you used the free trial, we keep one record that it
  was used: a one-way hash (SHA-256) of your lowercased email address and the date, not the address itself, so
  the trial is offered once per person.
- **If your subscription ends**, syncing stops. We email you a warning, and **30 days after it ends** we delete
  all mail metadata and credentials. We keep only your login and billing records so you can come back or
  resolve billing questions.
- Internal queues and records are cleaned up automatically: completed actions after 24 hours, failed ones and
  deletion records after 30 days.
- Stripe and Amazon keep their own records under their own policies and legal obligations (for example tax
  records).

[Lawyer review: retention for database backups, invoices and legal holds to be added.]

## Security

- Mailbox credentials are encrypted at rest; the key is held only by the sync service.
- Email HTML is treated as hostile: it is sanitised before display, scripts never run, and remote images are
  blocked until you allow them.
- Our services accept only authenticated requests. The sync service runs as a non-root user with a read-only
  filesystem and no shell.
- See [SECURITY.md](../SECURITY.md) to report a vulnerability.

## Your rights

You can ask to see, export, correct or delete your data. You can disconnect mailboxes and delete your account
yourself in the app's settings. For anything else, write to **sami@samihindi.com** and we will answer within
30 days. Depending on where you live
(for example in the EU, the UK, Switzerland or California) you may have further rights, including the right
to object to processing and to complain to your data protection authority.

[Lawyer review: legal bases for processing (GDPR art. 6), international transfer mechanisms, data locations,
and jurisdiction-specific sections to be added.]

## Children

Akatsuki is not directed at children under 16, and we do not knowingly collect their data.

## Self-hosted copies

If you run Akatsuki on your own hardware, you are in control of your data and this policy does not apply to
your copy: we receive nothing from it. The privacy of a self-hosted copy depends on how you set it up, for
example which database, AI providers and push settings you choose.

## Changes

If we change how we handle your data, we will update this page and change the date at the top. For material
changes we will tell signed-in users by email or in the app before the change takes effect.
