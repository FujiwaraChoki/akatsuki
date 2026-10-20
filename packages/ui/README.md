# @akatsuki/ui

Shared React primitives for the web app and the landing site, in the "First Light" direction
(docs/DECISIONS.md D-480–D-489): ink on paper, hairlines for structure, red only where something needs your eye
(the dawn marks and three brand buttons), Instrument Serif once per screen, Geist UI text, keyboard first. Styled **only** with `@akatsuki/tokens` CSS variables (a test fails on any colour literal or unknown
variable). Built on Radix primitives (MIT) for focus management and ARIA.

Mobile (React Native) does not use this package's components; it shares `errors.ts` copy
(`@akatsuki/ui/errors`, no React or DOM imports) and the tokens.

## Setup

```tsx
import "@akatsuki/tokens/tokens.css";
import "@akatsuki/tokens/fonts.css";
import "@akatsuki/ui/styles.css";
import { AkatsukiUiProvider } from "@akatsuki/ui";

<body class="ak-app">
  <AkatsukiUiProvider keymap={prefs.keymap} /* platform defaults to this browser */>
    <App />
  </AkatsukiUiProvider>
</body>
```

`AkatsukiUiProvider` = `UiConfigProvider` (platform and keymap for key hints) + `ToastProvider`. Theme follows the
tokens: light by default, dark under `prefers-color-scheme`, forced with `data-theme="light|dark"` on `<html>`.
React 19 is a peer dependency (pinned to the workspace version). Sources use `.ts`/`.tsx` import extensions;
the consuming tsconfig needs `"allowImportingTsExtensions": true` (Vite's default has it).

## Components

All accept `className` and the usual DOM props; refs are plain `ref` props (React 19).

| Component | Notes |
|---|---|
| `Button` | flat pills: `variant` `primary`/`ink` (foreground fill) · `brand` (accent fill; sign-in, onboarding, billing only) · `secondary` · `ghost` · `danger` (accent-text label); `size` `sm/md/lg`; `loading` (spinner, `aria-busy`, disabled); `shortcut` shows the key and sets `aria-keyshortcuts`; `icon`. `type="button"` by default. |
| `IconButton` | Required `label` (accessible name and tooltip), `icon`, `shortcut` (in the tooltip), `pressed` (`aria-pressed`), `tooltip={false}` to drop the tooltip. |
| `Kbd` | `shortcut="thread.archive"` (preferred) or raw `keys="cmd+k"`. Per platform; one keycap per key (`[Ctrl][K]`, `[G][I]`, macOS "⌘K"), read as "Ctrl+K" / "G then I". Renders nothing for palette-only commands. |
| `Tooltip` | `content`, optional `shortcut`, one focusable child. Opens on hover (400 ms) and immediately on focus. |
| `Menu`, `MenuTrigger`, `MenuContent`, `MenuItem`, `MenuCheckboxItem`, `MenuLabel`, `MenuSeparator`, `MenuGroup` | Radix dropdown: arrow keys, typeahead, Escape. `MenuItem` takes `shortcut`, `icon`, `destructive`, `onSelect`. |
| `Dialog` (+ `DialogTrigger`, `DialogClose`) | `title` (required, Geist 15/500; `hideTitle` keeps it for screen readers), `description`, `footer`, `size`; `anchor` + `width` make it an anchored popover (no scrim, flips above near the bottom). Focus trap, Escape, focus return. |
| `ToastProvider` / `useToast()` | `toast({ id?, title, description?, tone, action?, secondaryAction?, durationMs?, onDismiss? })` → id; `dismiss(id)`, `dismissAll()`. Same `id` replaces (op failures keyed by opId show once, §5.7). Errors are sticky and `role="alert"`; others `role="status"`, 4 s. Timers pause on hover/focus. `UNDO_TOAST_MS` = 10 s (§5.2); undo toasts show a hairline running out. `singleInfo` (or `AkatsukiUiProvider singleInfoToast`): one info toast at a time, at most two errors above it. An ink pill, bottom-left of the list pane (`--ak-toast-left/-bottom`). |
| `CommandPalette` | See below. |
| `List`, `ListRow`, `ListRowLeading`, `ListRowTitle`, `ListRowSnippet`, `ListRowMeta`, `ListRowTrailing`, `UnreadDot` | Listbox of options for dense mail lists. `ListRow` has independent `active` (j/k cursor, `aria-current`), `selected` (x), `unread`, `pending` states and fixed heights from the density tokens (`compact` 32 px, `comfortable` 44 px), so lists virtualize. Focus stays on `List` (`activeId` → `aria-activedescendant`). |
| `Avatar` | `name`, `email`, `src` (pass a proxied URL only), `size`. A mono letter tile (no tints); falls back to initials when the image fails. The mail views do not use it. |
| `Badge` | `count` (mono grey, capped "99+", zero renders nothing, like `mail.unreadCounts`) or a label child (a mono caps chip); `tone` `accent` (ink) / `neutral` / `outline`. |
| `Spinner` | `role="status"` with `label`; still (no rotation) under reduced motion. |
| `usePrefersReducedMotion()` | Live `prefers-reduced-motion`. (The orbs are gone, D-480.) |
| `Heading`, `Eyebrow`, `Crumb` | `Heading` `variant` `subject` (opened thread, 24 → 30 from 768px), `heading`, `empty`, `display`. `Eyebrow` is the sentence-case section label (12/500). `Crumb` is mono caps, for the reader crumb and the AI provenance line only. |

### Command palette

```tsx
const items = [
  ...paletteItemsFromShortcuts({ "thread.archive": archive, "go.inbox": goInbox, … }, { platform, keymap }),
  { id: "account:abc", label: "Switch to sami@…", section: "Accounts", hint: "Gmail", run: () => switchTo("abc") },
];
<CommandPalette open={open} onOpenChange={setOpen} items={items} recent={recentIds} onRun={remember} />
```

- `paletteItemsFromShortcuts(handlers, opts)` turns every `palette: true` command of `packages/shortcuts` that
  has a handler into an item with its section (`group`) and key hint, so the palette offers exactly what the
  screen can do and every keyboard action is reachable by name (BRIEF quality bar).
- Fuzzy matching on label, keywords and section (`fuzzyMatch`, `rankPaletteItems` are exported); matched
  characters are highlighted; sections keep input order while the query is empty, best match first otherwise;
  `recent` ids appear first under "Recent".
- Keys come from `packages/shortcuts` (`palette.next`: down / ctrl+n, `palette.prev`: up / ctrl+p,
  `palette.run`: Enter, `palette.close`: Escape), plus ctrl+Home / ctrl+End. Disabled items are skipped.
- ARIA: a modal dialog with a combobox input, `aria-activedescendant` into a listbox of grouped options.
- Opening it is the app's job (bind `palette.open`).

## Error copy (`errors.ts`, §5.7, §15)

```ts
import { describeOpFailure, actionVerb, errorCopy, errorMessage } from "@akatsuki/ui/errors";
toast({ id: f.opId, tone: "error",
  title: describeOpFailure({ verb: actionVerb(f.action), subject: f.subject, account: f.accountEmail, code: f.error.code }),
  action: { label: "Retry", onAction: () => retry({ opId: f.opId }) },
  secondaryAction: { label: "Dismiss", onAction: () => ack({ opId: f.opId }) } });
// "Couldn't archive 'Subject' in sami@…: the message no longer exists."
```

`ERROR_COPY` is a `Record<ErrorCode, { title, reason, remedy }>` typed against `@akatsuki/wire`'s registry, so a
new code does not compile until it has copy. `remedy` (`retry`, `reconnect`, `upgrade`, `settings`, `update`,
`sign_in`, `none`) tells the UI which button to offer. `errorMessage(thrown)` gives the title for any
WireError, ConvexError or AiFeatureError. `maskAccount("sami@x.com")` → `sami@…`.

## Styling rules

- Class prefix `ak-`, BEM-ish (`ak-row__title`, `ak-button--primary`). Override with `className`, not new colours.
- Only token variables. The roles in `styles.css` (SPEC §2.3) are opaque mixes of the foreground over the
  background: `--ak-fg-80/70/60` (secondary, tertiary, muted; muted steps up to tertiary on a surface),
  `--ak-line`, `--ak-line-strong`, `--ak-stroke`, `--ak-surface`, `--ak-surface-strong`; plus `--ak-dot`
  (`--dawn`), `--ak-on-accent`, `--ak-pop` (the level 1 shadow), `--ak-scrim`. `--ak-raised` and `--ak-shadow`
  remain as aliases.
- Focus is always visible: a 2px outline in the text colour (`--ak-focus-color`), 2px off the element (D-484).
- Flat, then one lift: only tooltips, menus, dialogs, the palette and toasts are raised; never a blur.
- Motion: product state changes are instant; overlays fade and rise 4px over 140ms; reduced motion keeps an 80ms
  fade and no transforms.

## Tests

`pnpm --filter @akatsuki/ui test`: Vitest + Testing Library on jsdom (keyboard flows of the palette, menus,
dialogs and toasts; ARIA roles; reduced motion; error copy for every code; the stylesheet's token-only rule).
