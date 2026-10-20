// One archive, on four clocks (§5.3, reworked in round 5). Not a second inbox: a single hairline
// with four stops, drawn in type. The first stop is the key press, marked with the red dot: the
// screen has already moved. The server render (no JS, reduced motion, crawlers, full-page
// captures) is the finished state; src/client/timing.ts plays it once, slowly, when it comes into
// view below the fold, and Replay plays it again.
import { icon } from "@/lib/inbox-render";

export const STOPS: { when: string; what: string; say: string }[] = [
  {
    when: "Key down",
    what: "You press E",
    say: "The change is made on your device first. Nothing waits for a server.",
  },
  {
    when: "Next frame",
    what: "Gone from your screen",
    say: "The thread leaves the list before the network is involved. Z puts it back.",
  },
  {
    when: "In the background",
    what: "Sent to your provider",
    say: "Akatsuki queues the change and the sync worker passes it to Gmail, Outlook or your IMAP server.",
  },
  {
    when: "Confirmed",
    what: "Your provider agrees",
    say: "If it refuses, the thread comes back to your inbox and you are told why.",
  },
];

export function Timeline() {
  return (
    <figure className="tl" data-timeline>
      <ol className="tl-track">
        {STOPS.map((s, i) => (
          <li key={s.when} className={`tl-stop is-done${i === 0 ? " is-key" : ""}`} data-tl-stop={i}>
            <p className="tl-when">{s.when}</p>
            <span className="tl-mark" aria-hidden="true" />
            <p className="tl-what">{s.what}</p>
            <p className="tl-say">{s.say}</p>
          </li>
        ))}
      </ol>
      <figcaption className="tl-cap">
        <span>
          Snooze, reminders and send later run on the server too, so a closed laptop still sends on&nbsp;time.
        </span>
        <button type="button" className="tlink tl-replay" data-tl-replay hidden>
          {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG icon from lib/inbox-render.ts, no user input */}
          Replay <span className="ico-wrap" dangerouslySetInnerHTML={{ __html: icon("replay") }} />
        </button>
      </figcaption>
    </figure>
  );
}
