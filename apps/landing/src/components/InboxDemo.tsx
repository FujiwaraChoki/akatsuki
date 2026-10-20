// The product mock (§4): semantic HTML rendered at build time from the fixture. Without JS it is
// a readable static inbox with t1 selected; src/client/inbox.ts makes every key work.
import { ACCOUNTS, LABELS, SNOOZE_OPTIONS, SPLITS, VIEWS } from "@/lib/inbox-fixture";
import {
  icon,
  inboxCount,
  initialState,
  paneHtml,
  rowsHtml,
  snoozedCount,
  splitCount,
} from "@/lib/inbox-render";
import { keysFor, SHORTCUTS, sequenceCaps } from "@/lib/shortcuts";
import { Kbd, Mod, ModText } from "./Kbd";

const Icon = ({ name, className = "" }: { name: string; className?: string }) => (
  // biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG icon from lib/inbox-render.ts, no user input
  <span className="ico-wrap" dangerouslySetInnerHTML={{ __html: icon(name, className) }} />
);

const STATIC_COUNTS: Record<string, string> = { drafts: "1" };

/** Palette commands inside the mock: every `palette: true` entry of the key map. */
const MOCK_COMMANDS = SHORTCUTS.filter((s) => s.palette);

export function InboxDemo() {
  const s = initialState();
  return (
    <div className="inbox-wrap container" id="inbox" data-inbox>
      <a className="skip skip-inbox" href="#after-inbox">
        Skip the interactive inbox
      </a>
      {/* The hero's one light: a horizon that is the product's own top edge. The product rises
          out of it on load; the light is brightest where it touches the frame. */}
      <div className="ib-horizon" aria-hidden="true">
        <span className="ib-band" />
        <span className="dawn-grain" />
        <span className="ib-line" />
      </div>
      <div className="ib-stage" data-stage>
        <div className="ib" data-frame>
          <div className="ib-side">
            <div className="ib-acct">
              All inboxes <Icon name="chevron" />
            </div>
            <nav className="ib-nav" aria-label="Views">
              {VIEWS.map((v) => {
                const count =
                  v.id === "inbox"
                    ? String(inboxCount(s))
                    : v.id === "snoozed"
                      ? String(snoozedCount(s))
                      : (STATIC_COUNTS[v.id] ?? "");
                return (
                  <button
                    key={v.id}
                    type="button"
                    className={`ib-navrow${v.id === "inbox" ? " is-active" : ""}`}
                    data-view={v.id}
                    tabIndex={-1}
                  >
                    <span className="ib-navlabel">{v.label}</span>
                    <span className="ib-navkeys" aria-hidden="true">
                      G {v.key}
                    </span>
                    <span className="ib-count" data-count={v.id}>
                      {count}
                    </span>
                  </button>
                );
              })}
            </nav>
            <p className="ib-side-label">Labels</p>
            <ul className="ib-labels">
              {LABELS.map((l) => (
                <li key={l.name}>
                  <span className="ib-lab-mark" aria-hidden="true" />
                  <span>{l.name}</span>
                  <span className="ib-count">{l.count}</span>
                </li>
              ))}
            </ul>
            <p className="ib-side-label">Accounts</p>
            <ul className="ib-accts">
              {ACCOUNTS.map((a) => (
                <li key={a.addr}>
                  <span className="ib-tile">{a.letter}</span>
                  <span className="ib-addr">{a.addr}</span>
                  <span className="ib-kind">{a.kind}</span>
                </li>
              ))}
            </ul>
            <div className="ib-side-foot">
              <span>
                <Mod />
                <Kbd>K</Kbd> Commands
              </span>
              <span>
                <Kbd>?</Kbd> Shortcuts
              </span>
            </div>
          </div>

          <div className="ib-bar">
            <div className="ib-tabs" role="tablist" aria-label="Splits" data-tabs>
              {SPLITS.map((sp) => (
                <button
                  key={sp}
                  type="button"
                  role="tab"
                  className="ib-tab"
                  aria-selected={sp === s.split}
                  data-split={sp}
                  tabIndex={sp === s.split ? 0 : -1}
                >
                  {sp}{" "}
                  <span className="ib-tabn" data-split-count={sp}>
                    {splitCount(s, sp)}
                  </span>
                </button>
              ))}
              <Kbd className="ib-tabkey">TAB</Kbd>
            </div>
            <div className="ib-bar-r">
              <button type="button" className="ib-hint" data-ib-search tabIndex={-1}>
                <Icon name="search" />
                <span className="ib-hint-t">Search</span> <Kbd>/</Kbd>
              </button>
              <button type="button" className="ib-hint" data-ib-compose tabIndex={-1}>
                <Icon name="compose" />
                <span className="ib-hint-t">Compose</span> <Kbd>C</Kbd>
              </button>
              <span className="ib-live" data-live-chip hidden>
                Live · keys go to this inbox
              </span>
            </div>
            <div className="ib-search" data-searchbox hidden>
              <Icon name="search" />
              <input
                className="ib-search-in"
                type="text"
                aria-label="Search the demo inbox"
                placeholder="from:, has:attachment, is:unread or any word"
                autoComplete="off"
                spellCheck={false}
                data-search-in
              />
              <span className="ib-search-ops" data-search-ops />
              <Kbd>Esc</Kbd>
            </div>
          </div>

          <div className="ib-list" data-list>
            <ul
              className="ib-rows"
              // biome-ignore lint/a11y/noNoninteractiveElementToInteractiveRole: the demo list is an ARIA listbox (aria-activedescendant), like the product's
              role="listbox"
              tabIndex={0}
              aria-label="Demo inbox, Important split"
              aria-activedescendant="ir-t1"
              data-rows
              // biome-ignore lint/security/noDangerouslySetInnerHtml: the first paint, rendered at build time from the typed fixture by the same pure renderer the client uses
              dangerouslySetInnerHTML={{ __html: rowsHtml(s) }}
            />
            <div className="ib-empty" data-empty hidden />
            <div className="ib-pop" role="menu" aria-label="Snooze until" data-pop hidden>
              <p className="ib-pop-head" data-pop-head>
                Snooze until
              </p>
              {SNOOZE_OPTIONS.map((o, i) => (
                <button
                  key={o.key}
                  type="button"
                  role="menuitem"
                  className={`ib-pop-row${i === 1 ? " is-hi" : ""}`}
                  data-snooze={o.key}
                  tabIndex={-1}
                >
                  <span>
                    {o.label}
                    {o.when && <span className="ib-pop-when">{o.when}</span>}
                  </span>
                  <Kbd>{o.key}</Kbd>
                </button>
              ))}
            </div>
          </div>

          <section className="ib-pane" aria-label="Reading pane">
            {/* biome-ignore lint/security/noDangerouslySetInnerHtml: the demo's first paint, rendered at build time from the typed fixture by the same pure renderer the client uses */}
            <div className="ib-pane-in" data-pane dangerouslySetInnerHTML={{ __html: paneHtml(s) }} />
            <div className="rp-reply" data-reply>
              <span data-reply-text>Reply…</span>
              <Kbd>R</Kbd>
            </div>
          </section>

          <div className="ib-status">
            <span className="ib-status-l" data-status>
              Important · {splitCount(s, "Important")} conversations
            </span>
            <span className="ib-hud" aria-hidden="true" data-hud />
            <span className="ib-sync">
              <i aria-hidden="true" />
              Synced · 3 accounts
            </span>
          </div>

          <div className="ib-sheet" data-isheet hidden>
            <button type="button" className="ib-sheet-back" data-isheet-back>
              ← Back <Kbd className="kbd-only">Esc</Kbd>
            </button>
            <div className="ib-pane-in" data-isheet-pane />
          </div>

          <div className="ib-palette" role="dialog" aria-label="Commands" data-ipal hidden>
            <input
              className="ib-pal-in"
              type="text"
              placeholder="Type a command…"
              aria-label="Type a command"
              autoComplete="off"
              spellCheck={false}
              data-ipal-in
            />
            <div className="ib-pal-list" role="listbox" aria-label="Commands" data-ipal-list>
              {MOCK_COMMANDS.map((c) => {
                const mac = keysFor(c, "mac")[0];
                const other = keysFor(c, "other")[0];
                return (
                  <button
                    key={c.id}
                    type="button"
                    role="option"
                    aria-selected="false"
                    className="ib-pal-row"
                    data-cmd-id={c.id}
                    data-label={c.label.toLowerCase()}
                    tabIndex={-1}
                  >
                    <span>{c.label}</span>
                    {other && mac && (
                      <span className="kseq">
                        {(mac === other
                          ? [["both", mac]]
                          : [
                              ["p-mac", mac],
                              ["p-other", other],
                            ]
                        ).map(([cls, k]) => (
                          <span key={cls} className={`kseq ${cls === "both" ? "" : cls}`}>
                            {sequenceCaps(k ?? "", cls === "p-mac" ? "mac" : "other")
                              .flat()
                              .map((cap, i) => (
                                // biome-ignore lint/suspicious/noArrayIndexKey: keycaps of one shortcut, a fixed sequence
                                <Kbd key={i}>{cap}</Kbd>
                              ))}
                          </span>
                        ))}
                      </span>
                    )}
                  </button>
                );
              })}
              <p className="ib-pal-empty" data-ipal-empty hidden>
                No command matches. Esc, then <ModText k="K" /> searches the site.
              </p>
            </div>
          </div>

          <form className="ib-compose" aria-label="New message" data-compose hidden>
            <p className="ib-compose-head">
              New message <Kbd>Esc</Kbd>
            </p>
            <label className="ib-compose-f">
              <span>To</span>
              <input type="text" defaultValue="lukas@brenner.design" autoComplete="off" data-compose-to />
            </label>
            <label className="ib-compose-f">
              <span>Subject</span>
              <input type="text" defaultValue="Thursday" autoComplete="off" />
            </label>
            <textarea
              className="ib-compose-body"
              rows={3}
              defaultValue={"Works for me. See you at 10:30."}
              aria-label="Message"
            />
            <div className="ib-compose-foot">
              <button type="submit" className="ib-send" data-send>
                Send{" "}
                <span className="kseq">
                  <Mod />
                  <Kbd>↵</Kbd>
                </span>
              </button>
              <button type="button" className="ib-later" data-send-later>
                Send later{" "}
                <span className="kseq">
                  <Mod />
                  <Kbd>⇧</Kbd>
                  <Kbd>L</Kbd>
                </span>
              </button>
            </div>
          </form>

          <p className="sr-only" aria-live="polite" data-live />
        </div>
      </div>
      <div className="ib-legend" id="after-inbox" tabIndex={-1}>
        <p className="ib-legend-keys">
          <span className="ib-legend-h">Try it</span>
          <span className="ib-lk">
            <Kbd>J</Kbd>
            <Kbd>K</Kbd> move
          </span>
          <span className="ib-lk">
            <Kbd>E</Kbd> archive
          </span>
          <span className="ib-lk">
            <Kbd>H</Kbd> snooze
          </span>
          <span className="ib-lk">
            <Kbd>S</Kbd> star
          </span>
          <span className="ib-lk">
            <Kbd>Z</Kbd> undo
          </span>
          <span className="ib-lk">
            <Mod />
            <Kbd>K</Kbd> everything else
          </span>
        </p>
        <p className="ib-tally js-only" aria-hidden="true" data-tally>
          0 keys · 0 clicks
        </p>
      </div>
      {/* Phones: the four keys as a touch bar on the selected row, plus the gestures. */}
      {/* biome-ignore lint/a11y/useSemanticElements: a set of related buttons; a fieldset would add form semantics and a legend */}
      <div className="touchbar js-only" role="group" aria-label="Act on the selected conversation">
        {(
          [
            ["e", "E", "archive"],
            ["h", "H", "snooze"],
            ["s", "S", "star"],
            ["z", "Z", "undo"],
          ] as const
        ).map(([k, g, l]) => (
          <button key={k} type="button" className="tb-key" data-touch={k}>
            <span className="tb-g" aria-hidden="true">
              {g}
            </span>
            <span className="tb-l">{l}</span>
          </button>
        ))}
      </div>
      <p className="touch-say">Swipe left to archive, right to snooze. Tap to open.</p>
    </div>
  );
}
