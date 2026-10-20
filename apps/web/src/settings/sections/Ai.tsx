// Settings: AI (ARCHITECTURE §12.2). Consent first (nothing runs before it, §12.7 rule 1). Then providers
// ("profiles"): Akatsuki managed (Pro), OpenRouter, Anthropic, OpenAI, Google, AI Gateway, Ollama and any
// OpenAI-compatible endpoint (LM Studio, llama.cpp, vLLM, a local Whisper server). Keys stay in this browser:
// memory for the session, or IndexedDB under a non-extractable WebCrypto key with "Remember on this browser"
// (D-009); `keyOnDevices` only lists device ids. Then one route per feature (or the default) with an optional
// model override, and the egress label under each row. Test connection runs from this device.
import { Button, errorMessage, useToast } from "@akatsuki/ui";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { keyStore } from "../../ai/keyStore.ts";
import { useAiModule } from "../../ai/module.ts";
import { visibleText } from "../../ai/thread.ts";
import { type AiFeatureId, useAi, useAiStream } from "../../ai/useAi.ts";
import { localVoice, setLocalVoice } from "../../ai/voice.ts";
import { useBackend, useBodies } from "../../backend/context.tsx";
import type { AiSettingsView, MeView } from "../../backend/types.ts";
import { deviceId } from "../../lib/ids.ts";
import { type TestResult, testConnection } from "../aiTest.ts";
import { Row, Section, Switch } from "../ui.tsx";

type Profile = AiSettingsView["profiles"][number];
type Route = AiSettingsView["routes"][number];
type Kind = Profile["kind"];

export const CONSENT_VERSION = 1;

const KINDS: { kind: Kind; label: string; hint: string; needsKey: boolean; needsModel: boolean }[] = [
  {
    kind: "akatsuki",
    label: "Akatsuki AI",
    hint: "Included with Pro. Zero data retention, never used for training.",
    needsKey: false,
    needsModel: false,
  },
  {
    kind: "ollama",
    label: "Ollama (local)",
    hint: "Runs on this computer. Nothing leaves the device.",
    needsKey: false,
    needsModel: true,
  },
  {
    kind: "openai_compatible",
    label: "OpenAI-compatible endpoint",
    hint: "LM Studio, llama.cpp, vLLM, a local Whisper server, or any OpenAI-style API.",
    needsKey: false,
    needsModel: true,
  },
  {
    kind: "openrouter",
    label: "OpenRouter",
    hint: "Your OpenRouter key. Requests ask for zero-data-retention routing.",
    needsKey: true,
    needsModel: false,
  },
  {
    kind: "anthropic",
    label: "Anthropic",
    hint: "Your Anthropic API key.",
    needsKey: true,
    needsModel: false,
  },
  {
    kind: "openai",
    label: "OpenAI",
    hint: "Your OpenAI API key. Also used for dictation.",
    needsKey: true,
    needsModel: true,
  },
  { kind: "google", label: "Google", hint: "Your Google AI Studio key.", needsKey: true, needsModel: true },
  {
    kind: "gateway",
    label: "Vercel AI Gateway",
    hint: "Your AI Gateway key.",
    needsKey: true,
    needsModel: false,
  },
];
const kindInfo = (k: Kind) => KINDS.find((x) => x.kind === k)!;

const FEATURES: { id: AiFeatureId | "dictation"; label: string }[] = [
  { id: "summarize", label: "Summaries" },
  { id: "instant_reply", label: "Suggested replies" },
  { id: "compose", label: "Write with AI" },
  { id: "rewrite", label: "Rewrite" },
  { id: "extract_actions", label: "Extract actions" },
  { id: "nl_search", label: "Search with AI" },
  { id: "triage", label: "Auto-triage" },
  { id: "voice_profile", label: "Writing style" },
  { id: "dictation", label: "Dictation" },
];

export function AiSection(_props: { me: MeView }) {
  const backend = useBackend();
  const m = backend.useMutations();
  const ai = useAi();
  const mod = useAiModule();
  const { toast } = useToast();
  const view = ai.view;
  const s = ai.settings;
  const consented = s.consent.grantedAt !== undefined;
  const [adding, setAdding] = useState<Kind | null>(null);

  const save = async (patch: Parameters<typeof m.saveAiSettings>[0]) => {
    if (mod) {
      const next = {
        ...s,
        ...patch,
        voiceProfile: patch.voiceProfile === null ? undefined : (patch.voiceProfile ?? s.voiceProfile),
      };
      if (next.voiceProfile === undefined) delete next.voiceProfile;
      const r = mod.zAiSettings.safeParse(next);
      if (!r.success) {
        toast({ title: r.error.issues[0]?.message ?? "Those settings are not valid.", tone: "error" });
        return false;
      }
    }
    try {
      await m.saveAiSettings(patch);
      return true;
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
      return false;
    }
  };

  const addProfile = async (p: Profile) => {
    const routes = s.routes.some((r) => r.feature === "default")
      ? s.routes
      : [...s.routes, { feature: "default" as const, profileId: p.profileId, enabled: true }];
    if (await save({ profiles: [...s.profiles, p], routes })) {
      setAdding(null);
      toast({ title: `${p.label} added.` });
    }
  };

  if (view === undefined) return null;
  return (
    <>
      <Section
        title="AI"
        description="Summaries, suggested replies, writing help, search and dictation. You choose the provider for each, including models on your own computer."
      >
        {!consented ? (
          <div className="ak-settings__consent">
            <p>
              AI features send only what each one needs, to the provider you pick: a thread for a summary,
              your draft for a rewrite, headers and previews for auto-triage. With Ollama or another local
              endpoint nothing leaves this computer. Akatsuki AI (Pro) uses zero-data-retention routing and
              nothing is stored or used for training; dictation through Akatsuki goes to OpenAI, which may
              keep audio up to 30 days for abuse monitoring. Each action shows where its data goes.
            </p>
            <Button
              variant="primary"
              onClick={() =>
                void save({
                  consent: { ...s.consent, grantedAt: Date.now(), version: CONSENT_VERSION },
                }).then((ok) => ok && toast({ title: "AI features are on." }))
              }
            >
              Turn on AI features
            </Button>
          </div>
        ) : (
          <>
            <Row label="AI features" hint="On. Turning them off stops every AI action on all your devices.">
              <Switch
                label="AI features"
                checked
                onChange={() =>
                  void save({ consent: { autoTriage: false, syncVoiceProfile: s.consent.syncVoiceProfile } })
                }
              />
            </Row>
            <Row
              label="Auto-triage new mail"
              hint="Sorts new inbox mail into categories and flags what needs a reply, from headers and previews only."
            >
              <Switch
                label="Auto-triage"
                checked={s.consent.autoTriage}
                onChange={(autoTriage) => void save({ consent: { ...s.consent, autoTriage } })}
              />
            </Row>
          </>
        )}
      </Section>

      <Section
        title="Providers"
        actions={
          <select
            className="ak-input ak-input--select"
            aria-label="Add a provider"
            value=""
            disabled={s.profiles.length >= 10}
            onChange={(e) => setAdding((e.target.value as Kind) || null)}
          >
            <option value="">Add a provider…</option>
            {KINDS.filter(
              (k) =>
                k.kind !== "akatsuki" || (ai.managedAi && !s.profiles.some((p) => p.kind === "akatsuki")),
            ).map((k) => (
              <option key={k.kind} value={k.kind}>
                {k.label}
              </option>
            ))}
          </select>
        }
      >
        {adding && (
          <NewProfile kind={adding} onCancel={() => setAdding(null)} onAdd={(p) => void addProfile(p)} />
        )}
        {s.profiles.length === 0 && !adding ? (
          <div className="ak-settings__emptystate">
            <p className="ak-settings__emptytext">
              <span className="ak-settings__row-label">No provider yet.</span>{" "}
              {ai.managedAi
                ? "Akatsuki AI is included with Pro and keeps nothing it reads. Or add Ollama to keep everything on this computer, or bring your own key."
                : "Add Ollama to keep everything on this computer, or bring your own key."}
            </p>
            {ai.managedAi && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() =>
                  void addProfile({
                    profileId: crypto.randomUUID(),
                    label: "Akatsuki AI",
                    kind: "akatsuki",
                    models: {},
                    keyOnDevices: [],
                  })
                }
              >
                Use Akatsuki AI
              </Button>
            )}
          </div>
        ) : (
          <ul className="ak-settings__cards">
            {s.profiles.map((p) => (
              <ProfileCard
                key={p.profileId}
                profile={p}
                onChange={(next) =>
                  void save({ profiles: s.profiles.map((x) => (x.profileId === p.profileId ? next : x)) })
                }
                onRemove={() =>
                  void save({
                    profiles: s.profiles.filter((x) => x.profileId !== p.profileId),
                    routes: s.routes.filter((r) => r.profileId !== p.profileId),
                  }).then((ok) => {
                    if (ok) void keyStore().remove(p.profileId);
                  })
                }
              />
            ))}
          </ul>
        )}
      </Section>

      {s.profiles.length > 0 && (
        <Section
          title="Which provider does what"
          description="The default applies to every feature you do not set."
        >
          <RoutesTable onSave={(routes) => void save({ routes })} />
        </Section>
      )}

      {consented && <VoiceSection onSync={(patch) => save(patch)} />}
    </>
  );
}

/** http to a non-loopback host: allowed by the settings schema, blocked by the browser on app.akatsuki.vc. */
function lanHttp(url: string): boolean {
  try {
    const u = new URL(url.trim());
    return u.protocol === "http:" && !/^(localhost|127(?:\.\d{1,3}){3}|\[::1\])$/i.test(u.hostname);
  } catch {
    return false;
  }
}

function NewProfile({
  kind,
  onAdd,
  onCancel,
}: {
  kind: Kind;
  onAdd: (p: Profile) => void;
  onCancel: () => void;
}) {
  const info = kindInfo(kind);
  const [label, setLabel] = useState(info.label);
  const [baseUrl, setBaseUrl] = useState(kind === "openai_compatible" ? "http://localhost:1234/v1" : "");
  const [model, setModel] = useState("");
  const mod = useAiModule();
  const problem =
    baseUrl.trim() && mod
      ? mod.baseUrlProblem(baseUrl.trim())
      : kind === "openai_compatible" && !baseUrl.trim()
        ? "needs a URL"
        : null;
  return (
    <form
      className="ak-settings__card ak-rule-form"
      aria-label={`Add ${info.label}`}
      onSubmit={(e) => {
        e.preventDefault();
        if (problem) return;
        onAdd({
          profileId: crypto.randomUUID(),
          label: label.trim() || info.label,
          kind,
          ...(baseUrl.trim() && kind !== "akatsuki" && { baseUrl: baseUrl.trim().replace(/\/+$/, "") }),
          models: model.trim() ? { default: model.trim(), fast: model.trim() } : {},
          keyOnDevices: [],
        });
      }}
    >
      <p className="ak-settings__hint">{info.hint}</p>
      <Row label="Name" htmlFor="np-label">
        <input id="np-label" className="ak-input" value={label} onChange={(e) => setLabel(e.target.value)} />
      </Row>
      {kind !== "akatsuki" && (
        <Row
          label="Server URL"
          htmlFor="np-url"
          hint={
            kind === "ollama"
              ? "Leave empty for http://127.0.0.1:11434."
              : kind === "openai_compatible"
                ? "Required."
                : "Optional, for proxies."
          }
        >
          <input
            id="np-url"
            className="ak-input ak-input--mono"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            aria-invalid={problem ? true : undefined}
          />
        </Row>
      )}
      {problem && (
        <p className="ak-compose__error" role="alert">
          The URL {problem}.
        </p>
      )}
      {!problem && lanHttp(baseUrl) && (
        <p className="ak-settings__hint" role="note">
          Browsers block plain http to other machines from an https page. Serve this endpoint over https, use
          it from the desktop app, or run it on this computer (localhost).
        </p>
      )}
      {info.needsModel && (
        <Row label="Model" htmlFor="np-model" hint="For example llama3.2, gpt-5.1-mini or gemini-2.5-flash.">
          <input
            id="np-model"
            className="ak-input ak-input--mono"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
        </Row>
      )}
      <div className="ak-settings__actions">
        <Button type="submit" variant="primary">
          Add provider
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function ProfileCard({
  profile: p,
  onChange,
  onRemove,
}: {
  profile: Profile;
  onChange: (p: Profile) => void;
  onRemove: () => void;
}) {
  const backend = useBackend();
  const keys = keyStore();
  useSyncExternalStore(keys.subscribe, keys.getVersion, keys.getVersion);
  const info = kindInfo(p.kind);
  const [key, setKey] = useState("");
  const [remember, setRemember] = useState(true);
  const [test, setTest] = useState<TestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [models, setModels] = useState(p.models);
  const me = deviceId();
  const hasKey = keys.has(p.profileId);
  const elsewhere = p.keyOnDevices.filter((d) => d !== me).length;
  const mod = useAiModule();
  useEffect(() => setModels(p.models), [p.models]);

  const saveKey = async () => {
    await keys.set(p.profileId, key, remember && keys.canRemember);
    setKey("");
    const on = new Set(p.keyOnDevices);
    if (remember && keys.canRemember) on.add(me);
    else on.delete(me);
    if (on.size !== p.keyOnDevices.length) onChange({ ...p, keyOnDevices: [...on] });
  };
  const removeKey = async () => {
    await keys.remove(p.profileId);
    if (p.keyOnDevices.includes(me)) onChange({ ...p, keyOnDevices: p.keyOnDevices.filter((d) => d !== me) });
  };

  return (
    <li className="ak-settings__card">
      {/* The model-picker anatomy (SPEC §3.8): a mono letter tile, the name, the host, where data goes. */}
      <div className="ak-settings__card-head ak-provider">
        <span className="ak-sidebar__tile ak-provider__tile" aria-hidden="true">
          {(p.label[0] ?? "?").toUpperCase()}
        </span>
        <div className="ak-provider__text">
          <span className="ak-settings__card-title">{p.label}</span>
          <span className="ak-settings__hint">
            {info.label}
            {p.baseUrl ? ` · ${p.baseUrl}` : ""}
          </span>
          {mod && <span className="ak-provider__egress">{mod.egressText(p, "default")}</span>}
        </div>
      </div>
      {info.needsKey && (
        <>
          <Row
            label="API key"
            htmlFor={`key-${p.profileId}`}
            hint={
              hasKey
                ? keys.isRemembered(p.profileId)
                  ? "Stored in this browser, encrypted with a key that cannot be exported."
                  : "Held in memory for this session only."
                : elsewhere > 0
                  ? `Key not set on this device. It is on ${elsewhere} other device${elsewhere === 1 ? "" : "s"}.`
                  : "Key not set on this device."
            }
          >
            <input
              id={`key-${p.profileId}`}
              className="ak-input ak-input--mono"
              type="password"
              autoComplete="off"
              placeholder={hasKey ? "••••••••" : "Paste your key"}
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </Row>
          <div className="ak-settings__actions">
            {keys.canRemember && (
              <label className="ak-check">
                <input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} />
                Remember on this browser
              </label>
            )}
            <Button size="sm" disabled={!key.trim()} onClick={() => void saveKey()}>
              Save key
            </Button>
            {hasKey && (
              <Button size="sm" variant="ghost" onClick={() => void removeKey()}>
                Remove key
              </Button>
            )}
          </div>
        </>
      )}
      {p.kind !== "akatsuki" && (
        <div className="ak-settings__models">
          {(["default", "fast", "transcription"] as const).map((tier) => (
            <Row
              key={tier}
              label={tier === "default" ? "Model" : tier === "fast" ? "Fast model" : "Dictation model"}
              htmlFor={`m-${tier}-${p.profileId}`}
              hint={
                tier === "fast"
                  ? "Triage and search."
                  : tier === "transcription"
                    ? "OpenAI or a Whisper server."
                    : undefined
              }
            >
              <input
                id={`m-${tier}-${p.profileId}`}
                className="ak-input ak-input--mono"
                value={models[tier] ?? ""}
                placeholder={
                  mod?.tableModel(tier, p.kind) ?? (tier === "transcription" ? "whisper-1" : "required")
                }
                onChange={(e) => setModels({ ...models, [tier]: e.target.value || undefined })}
                onBlur={() => {
                  const clean = Object.fromEntries(
                    Object.entries(models)
                      .filter(([, v]) => typeof v === "string" && v.trim())
                      .map(([k, v]) => [k, (v as string).trim()]),
                  );
                  if (JSON.stringify(clean) !== JSON.stringify(p.models)) onChange({ ...p, models: clean });
                }}
              />
            </Row>
          ))}
        </div>
      )}
      <div className="ak-settings__actions">
        <Button
          size="sm"
          loading={testing}
          onClick={async () => {
            setTesting(true);
            setTest(await testConnection(p, keys.get(p.profileId), backend.api));
            setTesting(false);
          }}
        >
          Test connection
        </Button>
        {test && (
          <span className={test.ok ? "ak-settings__ok" : "ak-compose__error"} role="status">
            {test.detail}
          </span>
        )}
        <span className="ak-compose__spacer" />
        <Button size="sm" variant="ghost" onClick={onRemove}>
          Remove provider
        </Button>
      </div>
    </li>
  );
}

function RoutesTable({ onSave }: { onSave: (routes: Route[]) => void }) {
  const ai = useAi();
  const s = ai.settings;
  const routeFor = (f: string) => s.routes.find((r) => r.feature === f);
  const set = (feature: Route["feature"], profileId: string, model?: string) => {
    const rest = s.routes.filter((r) => r.feature !== feature);
    if (!profileId) return onSave(rest);
    onSave([...rest, { feature, profileId, enabled: true, ...(model?.trim() && { model: model.trim() }) }]);
  };
  const rows: { id: Route["feature"]; label: string }[] = [{ id: "default", label: "Default" }, ...FEATURES];
  return (
    <table className="ak-routes">
      <thead>
        <tr>
          <th scope="col">Feature</th>
          <th scope="col">Provider</th>
          <th scope="col">Model</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const r = routeFor(row.id);
          const egress = ai.egress(row.id as never);
          return (
            <tr key={row.id}>
              <th scope="row">
                {row.label}
                <span className="ak-egress">
                  {egress ?? (row.id === "default" ? "Off" : "Same as default")}
                </span>
              </th>
              <td>
                <select
                  className="ak-input ak-input--select"
                  aria-label={`${row.label} provider`}
                  value={r?.profileId ?? ""}
                  onChange={(e) => set(row.id, e.target.value, r?.model)}
                >
                  <option value="">{row.id === "default" ? "Off" : "Same as default"}</option>
                  {s.profiles
                    .filter(
                      (p) =>
                        row.id !== "dictation" ||
                        ["akatsuki", "openai", "openai_compatible"].includes(p.kind),
                    )
                    .map((p) => (
                      <option key={p.profileId} value={p.profileId}>
                        {p.label}
                      </option>
                    ))}
                </select>
              </td>
              <td>
                <input
                  className="ak-input ak-input--mono"
                  aria-label={`${row.label} model`}
                  defaultValue={r?.model ?? ""}
                  disabled={!r}
                  placeholder="Provider default"
                  onBlur={(e) =>
                    r && e.target.value.trim() !== (r.model ?? "") && set(row.id, r.profileId, e.target.value)
                  }
                />
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function VoiceSection({
  onSync,
}: {
  onSync: (patch: { voiceProfile?: string | null; consent?: AiSettingsView["consent"] }) => Promise<boolean>;
}) {
  const ai = useAi();
  const backend = useBackend();
  const loader = useBodies();
  const sent = backend.useThreadList({ labelKey: "sys:sent" });
  const stream = useAiStream(ai);
  const { toast } = useToast();
  const s = ai.settings;
  const [text, setText] = useState(() => s.voiceProfile ?? localVoice() ?? "");
  const [building, setBuilding] = useState(false);
  const out = stream.state.object as { profile?: string } | null;
  useEffect(() => {
    if (stream.state.status === "done" && out?.profile) setText(out.profile);
  }, [stream.state.status, out?.profile]);
  const sentIds = useMemo(() => sent.rows.slice(0, 10).map((r) => r._id), [sent.rows]);

  const build = async () => {
    setBuilding(true);
    try {
      const samples: string[] = [];
      for (const id of sentIds) {
        const t = await backend.fetchThread(id);
        const mine = t && [...t.messages].reverse().find((x) => x.isOutbound && !x.isDraft);
        if (!mine) continue;
        const body =
          loader.cache.peek(mine._id) ?? (await backend.worker.getMessage(mine._id).catch(() => undefined));
        const v = visibleText(body, mine);
        if (v.trim()) samples.push(v.slice(0, 4000));
      }
      if (samples.length === 0) return toast({ title: "No sent mail to learn from yet." });
      const textVar = samples.map((x, i) => `Sample ${i + 1}:\n${x.trim()}`).join("\n\n");
      await stream.start("voice_profile", { samples: textVar });
    } catch (e) {
      toast({ title: errorMessage(e), tone: "error" });
    } finally {
      setBuilding(false);
    }
  };
  const save = async () => {
    const v = text.trim().slice(0, 2048);
    setLocalVoice(v || null);
    if (s.consent.syncVoiceProfile) await onSync({ voiceProfile: v || null });
    toast({ title: "Writing style saved." });
  };
  return (
    <Section
      title="Your writing style"
      description="Used by suggested replies and Write with AI. Describes style only, never quotes your mail. Kept on this device unless you sync it."
    >
      <textarea
        className="ak-textarea"
        rows={4}
        aria-label="Writing style"
        value={text}
        maxLength={2048}
        onChange={(e) => setText(e.target.value)}
        placeholder="Short, friendly sentences. Signs off with “Best”."
      />
      <Row
        label="Sync to my other devices"
        hint="Stores this description (not your mail) in your Akatsuki account."
      >
        <Switch
          label="Sync writing style"
          checked={s.consent.syncVoiceProfile}
          onChange={(syncVoiceProfile) =>
            void onSync({
              consent: { ...s.consent, syncVoiceProfile },
              ...(!syncVoiceProfile && { voiceProfile: null }),
              ...(syncVoiceProfile && text.trim() && { voiceProfile: text.trim().slice(0, 2048) }),
            })
          }
        />
      </Row>
      <div className="ak-settings__actions">
        <Button variant="primary" size="sm" onClick={() => void save()}>
          Save
        </Button>
        <Button
          size="sm"
          loading={building || stream.state.status === "running"}
          disabled={ai.status("voice_profile") !== "ready" || sentIds.length === 0}
          onClick={() => void build()}
        >
          Learn from my sent mail
        </Button>
      </div>
      <p className="ak-egress">{ai.egress("voice_profile") ?? ""}</p>
    </Section>
  );
}
