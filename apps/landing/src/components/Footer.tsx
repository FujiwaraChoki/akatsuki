import { SITE } from "@/lib/site";
import { ThemeControl } from "./Header";
import { Kbd, Mod } from "./Kbd";

export function FooterLinks() {
  return (
    <div className="grid foot-links">
      <p className="small foot-copy">
        <span className="foot-name">{SITE.name}</span> © 2026 {SITE.owner}
      </p>
      <div className="foot-col c1">
        <span className="foot-h">Product</span>
        <ul>
          <li>
            <a href="/pricing">Pricing</a>
          </li>
          <li>
            <a href="/download">Download</a>
          </li>
          <li>
            <a href="/changelog">Changelog</a>
          </li>
        </ul>
      </div>
      <div className="foot-col c2">
        <span className="foot-h">Trust</span>
        <ul>
          <li>
            <a href="/privacy">Privacy</a>
          </li>
          <li>
            <a href="/security">Security</a>
          </li>
          <li>
            <a href="/license">Licence</a>
          </li>
          <li>
            <a href="/terms">Terms</a>
          </li>
        </ul>
      </div>
      <div className="foot-col c3">
        <span className="foot-h">Site</span>
        <ul>
          <li>
            <button type="button" data-open-palette aria-label="Search">
              Search{" "}
              <span className="kseq" aria-hidden="true">
                <Mod />
                <Kbd>K</Kbd>
              </span>
            </button>
          </li>
          <li>
            <button type="button" data-open-shortcuts aria-label="Shortcuts">
              Shortcuts <Kbd>?</Kbd>
            </button>
          </li>
          <li>
            <ThemeControl />
          </li>
        </ul>
      </div>
    </div>
  );
}

/** Footer for every page except home, which ends in the sunrise instead. */
export function Footer() {
  return (
    <footer className="foot container">
      <FooterLinks />
      <p className="foot-legal">
        Open source, free for personal use · <span className="nowrap">ASAL-1.0</span> · No trackers on this
        site
      </p>
    </footer>
  );
}
