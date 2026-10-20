// AI: one component, blown up. A complete, crisply bordered reading pane, with the model picker
// open over its bottom-right corner, as it would be in the app. Each model names where it runs in
// plain words, never with a provider logo. src/client/picker.ts adds keys, the first-view
// step-through and the streamed summary. On phones the picker attaches to the summary's bottom edge.
import { Kbd } from "./Kbd";

const CHECK =
  '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3.5 8.5 6.5 11.5 12.5 4.5"/></svg>';

export const MODELS = [
  {
    id: "managed",
    where: "Akatsuki",
    name: "Akatsuki managed",
    desc: "Included with Pro",
    tag: "Sent to Akatsuki, not stored",
    short: "Akatsuki managed",
  },
  {
    id: "sonnet",
    where: "Your key",
    name: "Claude Sonnet 5.5",
    desc: "api.anthropic.com",
    tag: "Leaves this device · api.anthropic.com",
    short: "Claude Sonnet 5.5",
  },
  {
    id: "openrouter",
    where: "Your key",
    name: "Any model via OpenRouter",
    desc: "openrouter.ai",
    tag: "Leaves this device · openrouter.ai",
    short: "OpenRouter",
  },
  {
    id: "llama",
    where: "This laptop",
    name: "llama3.3",
    desc: "Ollama · localhost:11434",
    tag: "Stays on this device",
    short: "llama3.3",
  },
  {
    id: "lmstudio",
    where: "Your network",
    name: "LM Studio",
    desc: "192.168.1.20:1234",
    tag: "Stays on your local network · 192.168.1.20",
    short: "LM Studio",
  },
];
export const PICKED = 3;

export const AI_SUMMARY = [
  "Section 4 says the contract renews on 1 March; the order form says 1 April.",
  "Amara wants to know which date is right before she signs on Friday.",
];

const BODY = [
  "Hi, before I sign on Friday, could you check one thing?",
  "Section 4 has the renewal on 1 March, but the order form says 1 April.",
  "Which one should I go with?",
];

export function ModelPicker() {
  const m = MODELS[PICKED]!;
  return (
    <div className="mp-fig" data-picker>
      <article className="mp-card" aria-label="An opened conversation">
        <div className="mp-top">
          <p className="mlabel">Inbox · Important</p>
          <p className="mp-time">08:30</p>
        </div>
        <p className="mp-subj">Quick question about the contract</p>
        <p className="mp-from">Amara Okafor · to me</p>
        <div className="mp-sum">
          <p className="mlabel">
            Summary · <span data-mp-by>{m.short}</span>
          </p>
          <div className="mp-sum-text" data-mp-sum>
            {AI_SUMMARY.map((l) => (
              <p key={l}>{l}</p>
            ))}
          </div>
        </div>
        <div className="mp-body">
          {BODY.map((l) => (
            <p key={l}>{l}</p>
          ))}
        </div>
        <div className="mp-reply" aria-hidden="true">
          <span>Reply…</span>
          <Kbd>R</Kbd>
        </div>
      </article>
      <div className="mp">
        <div className="mp-head">
          <span>Summarise with</span>
          <span className="mp-keys" aria-hidden="true" data-mp-keys>
            <Kbd>⇧</Kbd>
            <Kbd>S</Kbd>
          </span>
        </div>
        <div
          className="mp-list"
          role="listbox"
          tabIndex={0}
          aria-label="Model for summaries"
          aria-activedescendant={`mp-${m.id}`}
          data-mp-list
        >
          {MODELS.map((x, i) => (
            // biome-ignore lint/a11y/useFocusableInteractive: focus stays on the listbox (aria-activedescendant)
            <div
              key={x.id}
              id={`mp-${x.id}`}
              role="option"
              aria-selected={i === PICKED}
              className={`mp-row${i === PICKED ? " is-sel" : ""}`}
              data-mp={i}
              data-tag={x.tag}
              data-short={x.short}
            >
              <span className="mp-tile" aria-hidden="true">
                {x.name.charAt(0).toUpperCase()}
              </span>
              <span className="mp-name">
                {x.name}
                <span className="mp-desc">{x.desc}</span>
              </span>
              <span className="mp-where">{x.where}</span>
              {/* biome-ignore lint/security/noDangerouslySetInnerHtml: static inline SVG icon from lib/inbox-render.ts, no user input */}
              <span className="mp-check" dangerouslySetInnerHTML={{ __html: CHECK }} />
            </div>
          ))}
        </div>
        <div className="mp-foot">
          <span className="mp-pill">
            <i aria-hidden="true" />
            <span data-mp-tag>{m.tag}</span>
          </span>
        </div>
      </div>
    </div>
  );
}
