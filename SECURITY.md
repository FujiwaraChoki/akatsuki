# Security policy

Akatsuki handles other people's email, so we take reports seriously and fix them first.

## How to report

Email **sami@samihindi.com** with a subject that starts with `SECURITY:`.

Please include:

- what the problem is and which component it affects;
- steps to reproduce, or a proof of concept;
- the impact you think it has;
- the commit, version or URL you tested;
- whether and how you would like to be credited.

**Do not** open a public issue, pull request, discussion or social media post about a vulnerability. Do not include real email content or other people's data in your report; use test accounts.

## What happens next

- We aim to acknowledge your report within 3 business days and to give you an initial assessment within 10 business days.
- We will keep you informed while we work on a fix and tell you when it is released.
- We will credit you in the release notes if you want to be credited.

Akatsuki is maintained by one person and is pre-alpha, so these are goals, not guarantees. There is no paid bug bounty at this time.

## Disclosure

**No public disclosure before a fix.** Please give us a reasonable time to release a fix, normally up to 90 days from your report, and agree the disclosure date with us. If a fix needs longer, we will explain why and agree a new date with you. If we stop responding, tell us in writing before you disclose.

## Scope

In scope:

- The code in this repository: the web, desktop and mobile apps, the sync worker (`apps/sync`), the API service (`apps/api`), the Convex functions (`convex/`), and the shared crates and packages.
- The hosted Akatsuki service on `akatsuki.vc` and its subdomains, `app.akatsuki.vc`, `sync.akatsuki.vc` and `api.akatsuki.vc`, tested only against accounts you own.

We especially want to hear about:

- bypasses of the HTML sanitiser, or any way to run script or load remote content in a message view;
- access to another user's data, mail metadata, message bodies or attachments;
- exposure of mailbox credentials, encryption keys, session tokens or AI keys;
- abuse of the image proxy, for example reaching internal addresses (SSRF) or using it as an open proxy;
- authentication, session or OAuth flaws, including the desktop sign-in flow;
- message text, prompts or AI output reaching storage or logs where the architecture says it must not;
- prompt injection in email that makes an AI feature leak data or take actions the user did not ask for;
- billing or entitlement bypasses.

Out of scope:

- vulnerabilities in third-party services and libraries that Akatsuki does not control (Gmail, Microsoft Graph, Convex, Stripe, AI providers); please report those to their owners, though we want to know if Akatsuki uses them unsafely;
- any system or service that is not part of Akatsuki, even if it shares infrastructure with it;
- denial of service and volumetric or load testing;
- social engineering, phishing and physical attacks;
- spam or abuse of the sign-up or email sending flows without a security impact;
- reports from automated scanners without a demonstrated impact;
- missing best-practice headers without a demonstrated impact;
- self-hosted copies you do not run yourself.

## Safe harbour

If you act in good faith, follow this policy, only use accounts and data you own, avoid privacy violations and service disruption, and stop as soon as you reach someone else's data, we will not pursue legal action against you for your research, and we will not ask anyone else to.

## Supported versions

Akatsuki is pre-alpha. Only the latest commit on `main` and the current hosted service receive security fixes. Self-hosters should stay on the latest `main`.
