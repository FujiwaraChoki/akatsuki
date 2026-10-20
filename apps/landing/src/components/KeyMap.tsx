// The key map on a quiet keyboard (round 5, after Linear's restraint): low-contrast caps with no
// legends, the keys that do something a shade brighter, and exactly one key lit. The caption under
// the board names the lit key's command. Pressing a real key, or clicking one, lights it. Phones
// get the spec's specimen list instead (§5.4): six ruled rows, a keycap and a serif verb each.
// Every action is checked against shortcuts.json at build time, so the map cannot drift.
import type { ReactNode } from "react";
import { glyphHtml } from "@/lib/glyphs";
import { shortcut } from "@/lib/shortcuts";
import { Cmd, Kbd } from "./Kbd";

const G = ({ c }: { c: string }) => (
  // biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG glyphs from lib/glyphs.ts, no user input
  <span className="gl-wrap" dangerouslySetInnerHTML={{ __html: glyphHtml(c) }} />
);

/** Readout copy per action: [shortcut id, verb, sentence]. */
export const ACTS: Record<string, [string, string, string]> = {
  back: ["nav.back", "Back", "Closes whatever is open, then takes you back to the list."],
  spam: ["thread.spam", "Report spam", "Shift 1 reports it and takes it out of every inbox you own."],
  trash: ["thread.trash", "Trash", "Shift 3 moves the conversation to Trash. Z brings it back."],
  split: [
    "split.next",
    "Next split",
    "Important, Calendar, Newsletters and Other, one key apart. Shift Tab goes back.",
  ],
  archive: [
    "thread.archive",
    "Archive",
    "Gone from the inbox at once and synced in the background. Z undoes it.",
  ],
  reply: [
    "thread.reply",
    "Reply",
    "Reply in place. Shift R suggests three replies you can edit before sending.",
  ],
  unread: ["thread.toggleRead", "Read or unread", "Mark it unread to come back to it later."],
  open: ["nav.open", "Open", "Opens the conversation. Enter does the same."],
  replyall: ["thread.replyAll", "Reply all", "Reply to everyone on the thread."],
  star: ["thread.star", "Star", "Star or unstar. Shift S summarises the conversation instead."],
  forward: ["thread.forward", "Forward", "Forward the conversation with its attachments."],
  goto: ["go.inbox", "Go to", "G then I, S, H, T or D: Inbox, Starred, Snoozed, Sent or Drafts."],
  snooze: [
    "thread.snooze",
    "Snooze",
    "It comes back when you said, even with your laptop closed. Shift H brings it back only if nobody replies.",
  ],
  next: ["nav.next", "Next", "The next conversation. The down arrow works too."],
  prev: ["nav.prev", "Previous", "The previous conversation. The up arrow works too."],
  label: ["thread.label", "Label", "Add or remove a label without opening the conversation."],
  undo: ["undo", "Undo", "Puts the last action back exactly where it was."],
  select: ["select.toggle", "Select", "Pick several conversations; E and H then act on all of them."],
  compose: ["compose.new", "Compose", "A new message, without leaving the inbox."],
  move: ["thread.move", "Move", "Move to a folder or label by typing its name."],
  search: [
    "search.open",
    "Search",
    "Operators like from:, has:attachment and is:unread. Shift / lists every shortcut.",
  ],
  palette: ["palette.open", "Everything else", "The command palette lists every action next to its key."],
  remind: ["thread.remind", "Remind me", "Shift H brings the conversation back only if nobody replies."],
  summarise: ["ai.summarize", "Summarise", "Shift S summarises the conversation with the model you chose."],
  suggest: [
    "ai.instantReply",
    "Suggest replies",
    "Shift R drafts three replies you can edit before sending.",
  ],
  help: ["help.shortcuts", "All shortcuts", "Lists every action, grouped, with the keys for your platform."],
};
// Fail the build if an action is not in the key map.
for (const [id] of Object.values(ACTS)) shortcut(id);

interface Key {
  /** Matching key (lower-case e.key, or a name). */
  k: string;
  g: ReactNode;
  u?: number;
  act?: string;
  /** Action when Shift is held. */
  sact?: string;
  legend?: string;
  cls?: string;
}

const L = (k: string, act?: string, legend?: string, sact?: string): Key => ({
  k,
  g: k.toUpperCase(),
  act,
  legend,
  sact,
});

const MOD_MAC = <Cmd />;

const ROWS: Key[][] = [
  [
    { k: "escape", g: "Esc", act: "back", legend: "back" },
    { k: "1", g: "!", sact: "spam", legend: "spam" },
    { k: "2", g: "2" },
    { k: "3", g: "#", sact: "trash", legend: "trash" },
    { k: "4", g: "4" },
    { k: "5", g: "5" },
    { k: "6", g: "6" },
    { k: "7", g: "7" },
    { k: "8", g: "8" },
    { k: "9", g: "9" },
    { k: "0", g: "0" },
    { k: "-", g: "-" },
    { k: "=", g: "=" },
    { k: "backspace", g: "Delete", u: 2, cls: "is-name" },
  ],
  [
    { k: "tab", g: "Tab", u: 1.5, act: "split", legend: "split", cls: "is-name" },
    L("q"),
    L("w"),
    L("e", "archive", "archive"),
    L("r", "reply", "reply", "suggest"),
    L("t"),
    L("y"),
    L("u", "unread", "unread"),
    L("i"),
    L("o", "open", "open"),
    L("p"),
    { k: "[", g: "[" },
    { k: "]", g: "]" },
    { k: "\\", g: "\\", u: 1.5 },
  ],
  [
    { k: "capslock", g: "Caps", u: 1.75, cls: "is-name" },
    L("a", "replyall", "reply all"),
    L("s", "star", "star", "summarise"),
    L("d"),
    L("f", "forward", "forward"),
    L("g", "goto", "go to"),
    L("h", "snooze", "snooze", "remind"),
    L("j", "next", "next"),
    L("k", "prev", "previous"),
    L("l", "label", "label"),
    { k: ";", g: ";" },
    { k: "'", g: "'" },
    { k: "enter", g: "Return", u: 2.25, act: "open", legend: "open", cls: "is-name" },
  ],
  [
    { k: "shift", g: "Shift", u: 2.25, cls: "is-name is-shift" },
    L("z", "undo", "undo"),
    L("x", "select", "select"),
    L("c", "compose", "compose"),
    L("v", "move", "move"),
    L("b"),
    L("n"),
    L("m"),
    { k: ",", g: "," },
    { k: ".", g: "." },
    { k: "/", g: "/", act: "search", legend: "search", sact: "help" },
    { k: "shift-r", g: "Shift", u: 2.75, cls: "is-name is-shift" },
  ],
];

/** Phone specimen rows (§5.4): [keys on the cap, act, verb shown, mod held]. */
const SPECIMENS: { keys: string[]; act: string; k: string; mod?: boolean }[] = [
  { keys: ["J", "K"], act: "next", k: "j" },
  { keys: ["E"], act: "archive", k: "e" },
  { keys: ["H"], act: "snooze", k: "h" },
  { keys: ["R"], act: "reply", k: "r" },
  { keys: ["/"], act: "search", k: "/" },
  { keys: ["mod", "K"], act: "palette", k: "k", mod: true },
];
/** A row also lights for its shifted or paired action. */
const ALSO: Record<string, string> = {
  next: "next prev",
  snooze: "snooze remind",
  reply: "reply suggest",
  search: "search help",
};
const SPECIMEN_VERB: Record<string, string> = {
  next: "Move",
  archive: "Archive",
  snooze: "Snooze",
  reply: "Reply",
  search: "Search",
  palette: "Everything else",
};
const SPECIMEN_SAY: Record<string, string> = {
  next: "Next and previous conversation. The arrow keys work too.",
  archive: "Gone from the inbox at once, synced in the background. Z undoes it.",
  snooze: "It comes back when you said, even with your laptop closed.",
  reply: "Shift R suggests three replies you can edit before sending.",
  search: "from:, has:attachment, before:, is:unread and the rest.",
  palette: "The command palette lists every action next to its key.",
};

function Cap(x: Key) {
  const units = x.u ?? 1;
  const mapped = !!(x.act || x.sact);
  return (
    <button
      type="button"
      tabIndex={-1}
      className={`km-key${mapped ? " is-mapped" : ""}${x.cls ? ` ${x.cls}` : ""}${x.k === "e" ? " is-lit" : ""}`}
      style={{ ["--u" as string]: units }}
      data-k={x.k}
      data-act={x.act}
      data-sact={x.sact}
      aria-label={x.legend ? `${typeof x.g === "string" ? x.g : x.k}: ${x.legend}` : undefined}
      aria-hidden={x.legend ? undefined : true}
    >
      {/* Keys that do nothing in Akatsuki are drawn, not written: their legend is decoration. */}
      {mapped || typeof x.g !== "string" ? (
        <span className="km-g">{x.g}</span>
      ) : (
        <span className="km-g" data-g={x.g} />
      )}
    </button>
  );
}

const Mod2 = ({ k, mac, other, u = 1.25 }: { k: string; mac: ReactNode; other: string; u?: number }) => (
  <button
    type="button"
    tabIndex={-1}
    className="km-key is-name"
    style={{ ["--u" as string]: u }}
    data-k={k}
    aria-hidden="true"
  >
    <span className="km-g">
      <span className="p-mac">{mac}</span>
      <span className="p-other" data-g={other} />
    </span>
  </button>
);

export function KeyMap() {
  return (
    <div className="km" data-keys>
      <div className="km-tray">
        {/* biome-ignore lint/a11y/useSemanticElements: a set of related buttons; a fieldset would add form semantics and a legend */}
        <div
          className="km-board"
          role="group"
          aria-label="The Akatsuki key map on a keyboard. Keys that do something are labelled."
        >
          {ROWS.map((row, i) => (
            <div key={row[0]?.k ?? i} className="km-row">
              {row.map((x) => (
                <Cap key={x.k} {...x} />
              ))}
            </div>
          ))}
          <div className="km-row">
            <Mod2 k="ctrl" mac={<G c="⌃" />} other="Ctrl" />
            <Mod2 k="alt" mac={<G c="⌥" />} other="Alt" />
            <Mod2 k="meta" mac={MOD_MAC} other="Super" />
            <button
              type="button"
              tabIndex={-1}
              className="km-key"
              style={{ ["--u" as string]: 5.75 }}
              data-k=" "
              aria-hidden="true"
            >
              <span className="km-g" />
            </button>
            <Mod2 k="meta-r" mac={MOD_MAC} other="Alt" />
            <Mod2 k="alt-r" mac={<G c="⌥" />} other="Ctrl" />
            <button
              type="button"
              tabIndex={-1}
              className="km-key is-arrow"
              style={{ ["--u" as string]: 1 }}
              data-k="left"
              aria-hidden="true"
            >
              <span className="km-g" data-g="←" />
            </button>
            <span className="km-stack" style={{ ["--u" as string]: 1 }}>
              <button
                type="button"
                tabIndex={-1}
                className="km-key is-arrow is-mapped"
                data-k="up"
                data-act="prev"
                aria-label="Up arrow: previous"
              >
                <span className="km-g">↑</span>
              </button>
              <button
                type="button"
                tabIndex={-1}
                className="km-key is-arrow is-mapped"
                data-k="down"
                data-act="next"
                aria-label="Down arrow: next"
              >
                <span className="km-g">↓</span>
              </button>
            </span>
            <button
              type="button"
              tabIndex={-1}
              className="km-key is-arrow"
              style={{ ["--u" as string]: 1 }}
              data-k="right"
              aria-hidden="true"
            >
              <span className="km-g" data-g="→" />
            </button>
          </div>
        </div>
      </div>

      <ul className="km-list" aria-label="Six of the keys">
        {SPECIMENS.map((x) => (
          <li key={x.act}>
            <button
              type="button"
              className={`km-spec${x.act === "archive" ? " is-lit" : ""}`}
              data-k={x.k}
              data-acts={ALSO[x.act] ?? x.act}
              data-mod={x.mod ? "" : undefined}
            >
              <span className="km-caps" aria-hidden="true">
                {x.keys.map((c) =>
                  c === "mod" ? (
                    <span key={c} className="km-cap">
                      <span className="p-mac">{MOD_MAC}</span>
                      <span className="p-other">Ctrl</span>
                    </span>
                  ) : (
                    <span key={c} className="km-cap">
                      {c}
                    </span>
                  ),
                )}
              </span>
              <span className="km-verb">{SPECIMEN_VERB[x.act]}</span>
              <span className="km-say">{SPECIMEN_SAY[x.act]}</span>
            </button>
          </li>
        ))}
      </ul>

      {/* The readout copy, for the client, as plain data on an inert element. */}
      <div
        hidden
        data-km-acts={JSON.stringify(
          Object.fromEntries(Object.entries(ACTS).map(([k, v]) => [k, [v[1], v[2]]])),
        )}
      />
    </div>
  );
}

/** The caption under the board: the lit key, its command and one sentence. */
export function KeyReadout() {
  const [, verb, say] = ACTS.archive!;
  return (
    <div className="kc" aria-live="polite" data-km-out>
      <p className="kc-chip">
        <span className="kc-keys" data-km-keys>
          <Kbd>E</Kbd>
        </span>
        <span className="kc-verb" data-km-verb>
          {verb}
        </span>
      </p>
      <p className="kc-say" data-km-say>
        {say}
      </p>
    </div>
  );
}
