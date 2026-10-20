// The waitlist has no backend, and says so by showing it: a compose window in the product's own
// style, previewing the note your mail app will open with. One address field. Nothing is sent from
// this page. Without JS the form is a plain mailto: action.
import { SITE, WL_LINE, WL_SUBJECT } from "@/lib/site";
import { Kbd } from "./Kbd";

export function Waitlist() {
  return (
    <div className="wl" data-wl>
      <form
        className="wl-mail"
        action={`mailto:${SITE.contact}?subject=${encodeURIComponent(WL_SUBJECT)}`}
        method="post"
        encType="text/plain"
        aria-label="Join the waitlist by email"
        data-wl-form
      >
        <p className="wl-head">New message</p>
        <div className="wl-f">
          <span className="wl-k">To</span>
          <span className="wl-v">
            Akatsuki <span className="wl-addr">{SITE.contact}</span>
          </span>
        </div>
        <div className="wl-f is-input">
          <label className="wl-k" htmlFor="wl-email">
            From
          </label>
          <input
            id="wl-email"
            className="wl-input"
            type="email"
            name="email"
            required
            autoComplete="email"
            inputMode="email"
            placeholder="Your email address"
            aria-describedby="wl-note"
            data-wl-input
          />
        </div>
        <div className="wl-f">
          <span className="wl-k">Subject</span>
          <span className="wl-v">{WL_SUBJECT}</span>
        </div>
        <p className="wl-body" aria-hidden="true">
          {WL_LINE}
        </p>
        <p className="wl-err" id="wl-err" role="alert" data-wl-err hidden>
          That address looks incomplete.
        </p>
        <div className="wl-foot">
          <button type="submit" className="btn btn-primary">
            Email us to join <Kbd className="kbd-only">↵</Kbd>
          </button>
          <p className="small wl-note" id="wl-note">
            Opens your mail app. Nothing is sent from this page.
          </p>
        </div>
      </form>
      <div className="wl-done" role="status" data-wl-done hidden>
        <p className="wl-done-t">
          Your mail app should have opened. If it didn’t, write to{" "}
          <span className="wl-done-addr">{SITE.contact}</span>.
        </p>
        <button type="button" className="tlink wl-copy" data-copy={SITE.contact}>
          Copy address
        </button>
      </div>
    </div>
  );
}
