import { Kbd, Mod } from "./Kbd";

const NAV = [
  { href: "/pricing", label: "Pricing" },
  { href: "/download", label: "Download" },
  { href: "/changelog", label: "Changelog" },
  { href: "/privacy", label: "Privacy" },
];

export function Header({ current }: { current: string }) {
  return (
    <>
      <a className="skip" href="#main">
        Skip to content
      </a>
      <header className="hdr" data-header>
        <div className="container hdr-in">
          <Logo />
          <div className="hdr-r">
            <nav className="hdr-nav" aria-label="Main">
              {NAV.map((n) => (
                <a key={n.href} href={n.href} aria-current={current === n.href ? "page" : undefined}>
                  {n.label}
                </a>
              ))}
            </nav>
            <span className="hdr-div" aria-hidden="true" />
            <button type="button" className="hdr-search" data-open-palette aria-label="Search">
              Search
              <span className="kseq" aria-hidden="true">
                <Mod />
                <Kbd>K</Kbd>
              </span>
            </button>
            <a className="btn btn-ghost is-sm" href="/#waitlist" data-waitlist>
              Join the waitlist
            </a>
            <button
              type="button"
              className="hdr-menu"
              data-open-menu
              aria-expanded="false"
              aria-controls="msheet"
            >
              Menu
            </button>
          </div>
        </div>
      </header>
      <div className="msheet" id="msheet" role="dialog" aria-modal="true" aria-label="Menu" hidden>
        <div className="msheet-top">
          <Logo />
          <button type="button" className="hdr-menu" style={{ display: "inline-flex" }} data-close-menu>
            Close
          </button>
        </div>
        <nav className="msheet-links" aria-label="Main">
          {NAV.map((n) => (
            <a key={n.href} href={n.href} aria-current={current === n.href ? "page" : undefined}>
              {n.label}
            </a>
          ))}
        </nav>
        <div className="msheet-foot">
          <a className="btn btn-primary" href="/#waitlist" data-waitlist data-close-menu>
            Join the waitlist
          </a>
          <ThemeControl />
        </div>
      </div>
    </>
  );
}

/** The logo: a red dot, nothing else. Its accessible name is the product's. */
export function Logo() {
  return (
    <a className="logo" href="/">
      <span className="logo-dot" aria-hidden="true" />
      <span className="sr-only">Akatsuki</span>
    </a>
  );
}

export function ThemeControl() {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a set of related buttons; a fieldset would add form semantics and a legend
    <div className="theme-ctl" role="group" aria-label="Theme" data-theme-ctl>
      <button type="button" data-theme-set="system" aria-pressed="true">
        Auto
      </button>
      <button type="button" data-theme-set="light" aria-pressed="false">
        Light
      </button>
      <button type="button" data-theme-set="dark" aria-pressed="false">
        Dark
      </button>
    </div>
  );
}
