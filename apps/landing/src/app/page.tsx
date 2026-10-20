import "./inbox.css";
import "./home.css";
import type { Metadata } from "next";
import { FooterLinks } from "@/components/Footer";
import { Header } from "@/components/Header";
import { InboxDemo } from "@/components/InboxDemo";
import { Kbd } from "@/components/Kbd";
import { KeyMap, KeyReadout } from "@/components/KeyMap";
import { ModelPicker } from "@/components/ModelPicker";
import { SpecSheet } from "@/components/SpecSheet";
import { Timeline } from "@/components/Timeline";
import { Waitlist } from "@/components/Waitlist";
import { SHORTCUT_COUNT, SHORTCUT_COUNT_WORD } from "@/lib/shortcuts";

export const metadata: Metadata = { alternates: { canonical: "/" } };

const PROVENANCE: [string, string][] = [
  ["Stays on this device", "A model running on this machine."],
  ["Stays on your local network", "A model server on your LAN, named by host."],
  ["Leaves this device", "Your own cloud provider, named, with what is sent."],
  ["Sent to Akatsuki, not stored", "Managed AI, included with Pro."],
];

// Five sections, no numbered eyebrows, each composed for what it shows, so no two share a layout:
// a wide headline over a hairline timeline (speed), a right-set headline over a quiet keyboard and
// its caption (keys), the product component with its argument beside it (AI), an inverted night
// band with the spec sheet (where your mail lives), one enormous number (price). Then the sunrise,
// where the red dot of the logo rises out of the horizon. Only the hero and the finale headlines
// end in a period, and that period is the dot.
export default function Home() {
  return (
    <>
      <Header current="/" />
      <main id="main">
        {/* ───────── hero ───────── */}
        <section className="hero" aria-labelledby="hero-title">
          <div className="container hero-in">
            <h1 id="hero-title" className="display hero-h1">
              <span className="hl hl-1">Clear your inbox</span>{" "}
              <span className="hl hl-2">
                by <em>first light</em>
                <span className="hero-dot">.</span>
              </span>
            </h1>
            <div className="hero-copy">
              <p className="lead">
                Akatsuki is an email client you drive from the keyboard. Gmail, Outlook and any IMAP account
                in one inbox, on the web, your desktop and your phone.
              </p>
              <div className="hero-cta">
                <a className="btn btn-primary btn-block-m" href="#waitlist" data-waitlist>
                  Join the waitlist
                </a>
                <a className="tlink is-down" href="#inbox" data-try-inbox>
                  Try the inbox <span className="arr">↓</span>
                </a>
              </div>
              <p className="hero-plat">In pre-alpha for the web, macOS, Linux, iPhone and Android.</p>
            </div>
          </div>
        </section>

        <section className="s-inbox" aria-label="A working demo inbox">
          <InboxDemo />
        </section>

        {/* ───────── speed: one archive on four clocks ───────── */}
        <section className="s s-speed" aria-labelledby="s1-title">
          <div className="container">
            <h2 id="s1-title" className="h2 sp-h2" data-reveal>
              <span className="ln">The screen moves on the key&nbsp;press,</span>{" "}
              <span className="ln tone">not on the network</span>
            </h2>
            <Timeline />
          </div>
        </section>

        {/* ───────── keys: a quiet keyboard, one key lit, its command named under it ───────── */}
        <section className="s s-keys" aria-labelledby="s2-title" id="keys">
          <div className="container">
            <h2 id="s2-title" className="h2 ks-h2" data-reveal>
              <span className="ln">A key for every action,</span>{" "}
              <span className="ln tone">the same on every&nbsp;device</span>
            </h2>
            <div className="ks-fig">
              <KeyMap />
              <KeyReadout />
            </div>
            <div className="ks-foot">
              <p className="small ks-note">
                {SHORTCUT_COUNT_WORD} actions live in one key map that the web, desktop and phone apps all
                read. Learn E once and it archives everywhere. Coming from Gmail? Switch to its keys
                in&nbsp;settings.
              </p>
              <button type="button" className="btn btn-ghost" data-open-shortcuts>
                All {SHORTCUT_COUNT} shortcuts
                <Kbd className="kbd-only">?</Kbd>
              </button>
            </div>
          </div>
        </section>

        {/* ───────── AI: the product component, with its argument beside it ───────── */}
        <section className="s s-ai" aria-labelledby="s3-title" id="ai">
          <div className="container grid ai-grid">
            <div className="ai-fig">
              <ModelPicker />
            </div>
            <div className="ai-text">
              <h2 id="s3-title" className="h2 is-s ai-h2" data-reveal>
                <span className="ln">Choose the model for each&nbsp;job,</span>{" "}
                <span className="ln tone">or keep it on your&nbsp;machine</span>
              </h2>
              <p className="body">
                Summaries, three suggested replies, rewrite, triage and dictation work with OpenRouter,
                Anthropic, OpenAI, Google, any OpenAI-compatible server, or Ollama on your laptop. Pick one
                for everything or one per feature. Nothing runs until you’ve said yes.
              </p>
              <dl className="prov">
                {PROVENANCE.map(([k, v]) => (
                  <div key={k} className="prov-row">
                    <dt>{k}</dt>
                    <dd>{v}</dd>
                  </div>
                ))}
              </dl>
              <p className="small">
                Every AI action shows one of these before it runs. Managed requests are never stored, logged
                or used for training.
              </p>
            </div>
          </div>
        </section>

        {/* ───────── where your mail lives: the night band ───────── */}
        <section className="s-mail night" aria-labelledby="s4-title">
          <div className="container">
            <div className="grid pv">
              <div className="pv-l">
                <h2 id="s4-title" className="h2 pv-h2">
                  <span className="line">Your mail stays</span>{" "}
                  <span className="line">with your provider</span>{" "}
                  <span className="line">and on your devices</span>
                </h2>
                <ul className="pv-links">
                  <li>
                    <a className="tlink" href="/privacy">
                      Privacy policy <span className="arr">→</span>
                    </a>
                  </li>
                  <li>
                    <a className="tlink" href="/security">
                      Security <span className="arr">→</span>
                    </a>
                  </li>
                  <li>
                    <a className="tlink" href="/license">
                      Licence <span className="arr">→</span>
                    </a>
                  </li>
                </ul>
              </div>
              <div className="pv-r">
                <SpecSheet mailOnly />
              </div>
            </div>
          </div>
        </section>

        {/* ───────── price: one number ───────── */}
        <section className="s s-price" aria-labelledby="s5-title">
          <div className="container">
            <div className="grid pz">
              <h2 id="s5-title" className="sr-only">
                Akatsuki Pro
              </h2>
              <p className="pz-l">
                <span className="price">$20</span>
                <span className="pz-per">a month</span>
              </p>
              <div className="pz-r">
                <p className="pz-lead">
                  Akatsuki Pro: hosted sync for every account, every app, and managed AI. The first 14 days
                  are free; starting the trial needs a card.
                </p>
                <a className="tlink" href="/pricing">
                  See everything in Pro <span className="arr">→</span>
                </a>
                <p className="pz-self">
                  Rather run it yourself? Self-hosting is free for personal,{" "}
                  <span className="nowrap">non-commercial</span> use.{" "}
                  <a className="tlink" href="/license">
                    How self-hosting works <span className="arr">→</span>
                  </a>
                </p>
                <p className="small pz-note">
                  Billed monthly in US dollars. Nothing is billed during pre-alpha.
                </p>
              </div>
            </div>
          </div>
        </section>
      </main>

      {/* ───────── finale: the sunrise. The red dot rises out of the horizon. ───────── */}
      <footer className="fin">
        <div className="container grid fin-in" id="waitlist">
          <div className="fin-l">
            <h2 className="display fin-h2">
              See you at{" "}
              <span className="nowrap">
                dawn<span className="hero-dot">.</span>
              </span>
            </h2>
            <p className="fin-copy">
              Akatsuki is in pre-alpha. Leave your address and we’ll write once, when there is something you
              can use.
            </p>
          </div>
          <div className="fin-r">
            <Waitlist />
          </div>
        </div>
        <div className="sun" data-sunrise aria-hidden="true">
          <span className="sun-glow" />
          <span className="dawn-grain" />
          <span className="sun-disc" />
          <span className="sun-line" />
        </div>
        <div className="container fin-ground">
          <FooterLinks />
          <p className="fin-legal">
            <span className="nowrap">Open source, free for personal use</span> ·{" "}
            <span className="nowrap">ASAL-1.0</span> ·{" "}
            <span className="nowrap">No trackers on this site</span>
          </p>
        </div>
      </footer>
    </>
  );
}
