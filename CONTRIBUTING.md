# Contributing to Akatsuki

Thank you for helping. Akatsuki is **open source, free for personal use**: it is licensed under the Akatsuki Source-Available License (ASAL-1.0) in [`LICENSE`](LICENSE), and commercial use, including use by any organisation, needs written permission from the copyright holder (sami@samihindi.com). The ASAL-1.0 is not an OSI-approved licence, because it reserves commercial use. Before you send anything, read the section on the rights you grant. They are broad, and you must agree to them for a contribution to be accepted.

## The rights you grant

When you submit a contribution (code, documentation, design, tests, fixtures, or issue text), you keep your copyright. You also grant Sami Hindi, the copyright holder, and anyone the copyright holder authorises:

- a **perpetual, worldwide, non-exclusive, royalty-free, irrevocable** licence to use, copy, modify, publish, distribute and sublicense your contribution and works based on it;
- the **right to relicense** your contribution under any terms, **including commercial and proprietary terms**, for example to sell commercial licences to Akatsuki or to run Akatsuki Pro;
- a matching patent licence for any patent you can license that your contribution necessarily infringes.

You confirm that the contribution is your own work or that you have the right to grant these licences, including your employer's permission where needed. To the extent the law allows, you waive moral rights in the contribution against the copyright holder. The copyright holder does not have to accept, use or credit any contribution.

The binding wording is section 6 of [`LICENSE`](LICENSE). If you do not agree with it, please do not contribute.

## Sign-off (required on every commit)

Every commit must carry a `Signed-off-by` line with your real name and an email address you can be reached at:

```
Signed-off-by: Your Name <you@example.com>
```

`git commit -s` adds it for you. By signing off you certify the **Akatsuki Contributor Certificate**:

> By making this contribution, I certify that:
>
> (a) I created the contribution in whole or in part and I have the right to submit it under the terms of section 6 of the Akatsuki Source-Available License; or
>
> (b) the contribution is based on earlier work that, to the best of my knowledge, is covered by a licence that lets me submit it, with my changes, under those terms, and I have said clearly which parts come from that work and under which licence; or
>
> (c) the contribution was given to me directly by someone who certified (a) or (b), and I have not changed it.
>
> (d) I understand and agree that this project and the contribution are public, that a record of the contribution, including my sign-off and personal details I submit with it, is kept indefinitely, and that the copyright holder may relicense the contribution, including under commercial terms.

Pull requests with unsigned commits are not merged. To sign off commits you already made, run `git rebase --signoff main` and force-push your branch.

## Workflow

1. **Talk first for anything large.** Open an issue describing the problem and your proposed change before writing a big patch. Small fixes can go straight to a pull request.
2. **Read the contract.** [`docs/BRIEF.md`](docs/BRIEF.md) is the source of truth, then [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), then the code. The names, types and wire shapes in the architecture are exact. If your change needs a different contract, edit `docs/ARCHITECTURE.md` and add an entry to [`docs/DECISIONS.md`](docs/DECISIONS.md) in the same pull request, with the alternatives and the reason. Never edit a past decision; add a new one that supersedes it.
3. **Fork and branch.** Branch from `main` with a short descriptive name, for example `fix/imap-idle-reconnect`.
4. **Set up.** Follow [`docs/DEV.md`](docs/DEV.md) for the toolchain, install and local services.
5. **Make the change, with tests.** Shared logic has TypeScript and Rust twins that must pass the same fixtures (architecture §1.7). If you change one side, change the other and the fixtures in the same pull request.
6. **Run every check locally** before you push. From the repository root:

   ```sh
   pnpm lint          # Biome over the whole TypeScript tree
   pnpm typecheck
   pnpm test
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   `pnpm format` fixes formatting. [`docs/DEV.md`](docs/DEV.md) has the full list, including the checks that need local services. CI runs the same checks and a failure blocks the merge.
7. **Update status.** If your change moves a component forward, update its row in [`docs/STATUS.md`](docs/STATUS.md).
8. **Open a pull request** that says what changed, why, and how you tested it. Keep pull requests focused; one concern per pull request.

## Rules that reviewers enforce

- **No secrets, ever.** Never commit keys, tokens, passwords or `.env` files. Document variable names in `.env.example` only. Real values go in your own gitignored `.env.local`.
- **No real email.** Fixtures and tests use synthetic messages only. Never paste real mail, addresses or headers from a real mailbox into an issue, test or log.
- **Message text stays out of Convex and out of logs.** Logs may contain ids, counts, timings and error codes only: never subjects, snippets, addresses, bodies, prompts, AI output, tokens or query strings.
- **Email HTML is hostile input.** Everything goes through the one sanitiser in `crates/mime`. No scripts, no forms, no `javascript:` URLs, and remote images stay blocked by default.
- **Tests do not need real keys.** Mock providers, or use Ollama or recorded fixtures.
- **Keyboard first.** New actions get a shortcut in `packages/shortcuts` and an entry in the command palette.
- **Accessibility.** Visible focus states, labelled controls, and `prefers-reduced-motion` respected.
- **Design tokens.** Colours, type and spacing come from `packages/tokens`. Nothing else hard-codes the palette.
- **Copy.** Plain and direct. No emoji as decoration. Describe Akatsuki as "open source, free for personal use", always with the commercial-use caveat nearby: commercial use, including use by any organisation, needs written permission. The licence keeps its name, the Akatsuki Source-Available License (ASAL-1.0).
- **Pinned versions.** The packages listed in architecture §0 are pinned exactly. Do not loosen them.
- **New dependencies** need a licence compatible with the ASAL-1.0 and an entry in [`THIRD_PARTY.md`](THIRD_PARTY.md).

## Commit messages

Write the subject in the imperative, under about 72 characters, with an optional scope: `sync: retry Graph delta on 410`. Explain the why in the body when it is not obvious. End with your `Signed-off-by` line.

## Security issues

Do not open a public issue for a vulnerability. Follow [`SECURITY.md`](SECURITY.md).

## Conduct

Everyone taking part follows the [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).
