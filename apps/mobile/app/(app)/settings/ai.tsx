// AI provider settings (§12.2, §12.7): consent, the provider used for AI features, BYO keys kept only in this
// phone's secure storage, and a clear label of where data goes for each provider.

import type { AiFeature, AiProfile, AiSettings, ProviderKind } from "@akatsuki/ai";
import { api } from "@akatsuki/convex";
import { useMutation, useQuery } from "convex/react";
import { router } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, View } from "react-native";
import { ActionMenu } from "../../../src/components/ActionMenu";
import { EgressLine } from "../../../src/components/Egress";
import { Header, HeaderButton } from "../../../src/components/Header";
import { SheetModal } from "../../../src/components/SheetModal";
import { Button, Field, Label, Row, Screen, Section, Toggle, Txt } from "../../../src/components/ui";
import { testConnection } from "../../../src/lib/aiTest";
import { getDeviceId } from "../../../src/lib/device";
import { errorText } from "../../../src/lib/errors";
import { newUuidV4 } from "../../../src/lib/ids";
import { SECURE_STORE_NAME, secureDelete, secureGet, secureSet } from "../../../src/lib/secure";
import { recordAiKey } from "../../../src/lib/session";
import { useShortcuts } from "../../../src/state/keyboard";
import { useMail } from "../../../src/state/mail";
import { showToast } from "../../../src/state/toasts";
import { useTheme } from "../../../src/theme/ThemeProvider";
import {
  CONSENT_VERSION,
  canTranscribe,
  consentText,
  defaultModel,
  defaultProfile,
  draftFromProfile,
  draftProblems,
  egressView,
  FEATURE_ROWS,
  featureRoute,
  featureRouteSummary,
  keyStatus,
  keyStatusText,
  keyStorageName,
  modelRequired,
  newDraft,
  PROVIDER_CHOICES,
  type ProfileDraft,
  profileFromDraft,
  providerChoice,
  removeProfile,
  routeChoices,
  setFeatureRoute,
  settingsOrEmpty,
  upsertProfile,
  withKeyOnDevice,
} from "../../../src/vm/aiSettings";

function EgressBadge({ text }: { text: string; local?: boolean }) {
  return <EgressLine text={text} full />;
}

function ProfileEditor({
  draft: initial,
  existing,
  isDefault,
  onClose,
  settings,
  managedAiAllowed,
  deviceId,
}: {
  draft: ProfileDraft;
  existing?: AiProfile;
  isDefault: boolean;
  onClose: () => void;
  settings: AiSettings;
  managedAiAllowed: boolean;
  deviceId: string;
}) {
  const t = useTheme();
  const save = useMutation(api.ai.saveSettings);
  const [d, setD] = useState(initial);
  const [key, setKey] = useState("");
  const [hasLocalKey, setHasLocalKey] = useState(false);
  const [makeDefault, setMakeDefault] = useState(isDefault || settings.profiles.length === 0);
  const [problems, setProblems] = useState<string[]>([]);
  const [test, setTest] = useState<{ busy: boolean; result?: string; ok?: boolean }>({ busy: false });
  const [saving, setSaving] = useState(false);
  const choice = providerChoice(d.kind);

  useEffect(() => {
    secureGet(keyStorageName(d.profileId))
      .then((v) => setHasLocalKey(!!v))
      .catch(() => {});
  }, [d.profileId]);

  const preview = profileFromDraft(d, existing);
  const egress = egressView(preview);

  const runTest = async () => {
    setTest({ busy: true });
    const k = key.trim() || (await secureGet(keyStorageName(d.profileId))) || undefined;
    const r = await testConnection(preview, k);
    setTest({ busy: false, ok: r.ok, result: r.ok ? r.detail : r.error });
  };

  const submit = async () => {
    const p = draftProblems(d, { managedAiAllowed });
    if (choice.needsKey && !key.trim() && !hasLocalKey && !existing?.keyOnDevices.length)
      p.push("Paste your API key.");
    setProblems(p);
    if (p.length > 0) return;
    setSaving(true);
    try {
      const name = keyStorageName(d.profileId);
      if (key.trim()) {
        await secureSet(name, key.trim());
        await recordAiKey(name, true);
      }
      const stored = key.trim().length > 0 || hasLocalKey;
      const profile = choice.needsKey ? withKeyOnDevice(preview, deviceId, stored) : preview;
      const next = upsertProfile(settings, profile, makeDefault);
      await save({ profiles: next.profiles, routes: next.routes });
      setKey("");
      onClose();
      showToast({ id: "ai-saved", text: `${profile.label} saved` });
    } catch (err) {
      setProblems([errorText(err)]);
    } finally {
      setSaving(false);
    }
  };

  const remove = async () => {
    try {
      const name = keyStorageName(d.profileId);
      await secureDelete(name);
      await recordAiKey(name, false);
      const next = removeProfile(settings, d.profileId);
      await save({ profiles: next.profiles, routes: next.routes });
      onClose();
    } catch (err) {
      setProblems([errorText(err)]);
    }
  };

  const forgetKey = async () => {
    const name = keyStorageName(d.profileId);
    await secureDelete(name);
    await recordAiKey(name, false);
    setHasLocalKey(false);
    if (existing) {
      const next = upsertProfile(settings, withKeyOnDevice(existing, deviceId, false), false);
      await save({ profiles: next.profiles, routes: next.routes }).catch(() => {});
    }
  };

  return (
    <SheetModal visible presentationStyle="pageSheet" onRequestClose={onClose}>
      <ScrollView
        style={{ backgroundColor: t.colors.background }}
        contentContainerStyle={{ padding: t.space[2], gap: t.space[2], paddingBottom: t.space[6] }}
        keyboardShouldPersistTaps="handled"
      >
        <Txt variant="title">{choice.title}</Txt>
        <Txt variant="body" tone="secondary">
          {choice.description}
        </Txt>
        <EgressBadge text={egress.text} local={egress.egress === "device" || egress.egress === "network"} />
        <Field label="Name" value={d.label} onChangeText={(v) => setD({ ...d, label: v })} />
        {choice.baseUrl !== "none" ? (
          <Field
            label={d.kind === "ollama" ? "Ollama address" : "Server address"}
            value={d.baseUrl}
            onChangeText={(v) => setD({ ...d, baseUrl: v })}
            placeholder={choice.baseUrlPlaceholder}
            autoCapitalize="none"
            autoCorrect={false}
            keyboardType="url"
          />
        ) : null}
        {choice.needsKey || d.kind === "openai_compatible" ? (
          <View style={{ gap: t.space["0_5"] }}>
            <Field
              label={choice.needsKey ? "API key" : "API key (optional)"}
              value={key}
              onChangeText={setKey}
              placeholder={hasLocalKey ? "Stored on this phone. Paste to replace." : choice.keyPlaceholder}
              secureTextEntry
              autoCapitalize="none"
              autoCorrect={false}
              textContentType="password"
            />
            <Txt variant="meta" tone="muted">
              Kept only in {SECURE_STORE_NAME}. It is never sent to Akatsuki or synced to your other devices.
            </Txt>
            {hasLocalKey ? (
              <Button title="Remove the key from this phone" variant="ghost" compact onPress={forgetKey} />
            ) : null}
          </View>
        ) : null}
        {d.kind !== "akatsuki" ? (
          <>
            <Field
              label={modelRequired(d.kind) ? "Model" : "Model (optional)"}
              value={d.model}
              onChangeText={(v) => setD({ ...d, model: v })}
              placeholder={defaultModel(d.kind, "default") ?? (d.kind === "ollama" ? "llama3.3" : "model id")}
              autoCapitalize="none"
              autoCorrect={false}
            />
            <Field
              label="Fast model for triage and search (optional)"
              value={d.fastModel}
              onChangeText={(v) => setD({ ...d, fastModel: v })}
              placeholder={defaultModel(d.kind, "fast") ?? "same as above"}
              autoCapitalize="none"
              autoCorrect={false}
            />
            {canTranscribe(d.kind) ? (
              <Field
                label={
                  d.kind === "openai"
                    ? "Dictation model (optional)"
                    : "Dictation model (for a Whisper server)"
                }
                value={d.transcriptionModel}
                onChangeText={(v) => setD({ ...d, transcriptionModel: v })}
                placeholder={defaultModel(d.kind, "transcription") ?? "whisper-1"}
                autoCapitalize="none"
                autoCorrect={false}
              />
            ) : null}
          </>
        ) : null}
        <Row
          title="Use for all AI features"
          right={
            <Toggle
              accessibilityLabel="Use for all AI features"
              value={makeDefault}
              onValueChange={setMakeDefault}
            />
          }
        />
        <Button title="Test connection" busy={test.busy} onPress={runTest} />
        {test.result ? (
          <Txt variant="meta" tone={test.ok ? "secondary" : "error"} accessibilityLiveRegion="polite">
            {test.result}
          </Txt>
        ) : null}
        {problems.map((p) => (
          <Txt key={p} variant="meta" tone="error">
            {p}
          </Txt>
        ))}
        <Button title="Save" variant="primary" busy={saving} onPress={submit} />
        {existing ? <Button title="Remove this provider" variant="ghost" onPress={remove} /> : null}
        <Button title="Cancel" variant="ghost" onPress={onClose} />
      </ScrollView>
    </SheetModal>
  );
}

export default function AiSettingsScreen() {
  const t = useTheme();
  const mail = useMail();
  const view = useQuery(api.ai.getSettings, {});
  const settings = settingsOrEmpty(view);
  const save = useMutation(api.ai.saveSettings);
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [localKeys, setLocalKeys] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<{ draft: ProfileDraft; existing?: AiProfile } | null>(null);
  const [picker, setPicker] = useState(false);
  const [routing, setRouting] = useState<AiFeature | null>(null);
  const managedAiAllowed = mail.me?.entitlement.managedAi === true;
  const current = defaultProfile(settings);
  useShortcuts("list", { "nav.back": () => router.back() });

  useEffect(() => {
    getDeviceId().then(setDeviceId);
  }, []);
  // Re-read after the editor closes (`editing`), since it may have stored or removed a key.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `editing` is the trigger
  useEffect(() => {
    let alive = true;
    Promise.all(
      settings.profiles.map(
        async (p) =>
          [p.profileId, !!(await secureGet(keyStorageName(p.profileId)).catch(() => null))] as const,
      ),
    ).then((pairs) => {
      if (alive) setLocalKeys(Object.fromEntries(pairs));
    });
    return () => {
      alive = false;
    };
  }, [settings.profiles, editing]);

  const consented = settings.consent.grantedAt !== undefined;
  const setConsent = (on: boolean) => {
    const { grantedAt: _g, version: _v, ...rest } = settings.consent;
    const consent = on
      ? { ...rest, grantedAt: Date.now(), version: CONSENT_VERSION }
      : { ...rest, autoTriage: false };
    save({ consent }).catch((err) => showToast({ id: "ai-consent", text: errorText(err), tone: "error" }));
  };

  return (
    <Screen>
      <Header title="AI" onBack={() => router.back()} />
      <ScrollView contentContainerStyle={{ paddingBottom: t.space[8] }}>
        <Section
          title="Permission"
          footer={
            current
              ? consentText(current)
              : "Choose a provider below. Nothing is sent anywhere until you allow AI and pick one."
          }
        >
          <Row
            title="Allow AI features"
            detail="Summaries, suggested replies, writing help and smart search. Each runs only when you use it."
            right={
              <Toggle accessibilityLabel="Allow AI features" value={consented} onValueChange={setConsent} />
            }
          />
          <Row
            title="Sort new mail automatically"
            detail="Sends the sender, subject and preview of new mail to the provider to sort it into splits."
            right={
              <Toggle
                accessibilityLabel="Sort new mail automatically"
                value={settings.consent.autoTriage}
                disabled={!consented}
                onValueChange={(v) => {
                  save({ consent: { ...settings.consent, autoTriage: v } }).catch(() => {});
                }}
              />
            }
          />
        </Section>

        <Section title="Providers">
          {settings.profiles.length === 0 ? <Row title="No provider yet" /> : null}
          {settings.profiles.map((p) => {
            const egress = egressView(p);
            const status = deviceId ? keyStatus(p, deviceId, localKeys[p.profileId] === true) : "not_needed";
            return (
              <View
                key={p.profileId}
                style={{
                  paddingHorizontal: t.space[2],
                  paddingVertical: t.space["1_5"],
                  gap: t.space["0_5"],
                }}
              >
                <View style={{ flexDirection: "row", alignItems: "center", gap: t.space[1] }}>
                  <Txt variant="bodyStrong" style={{ flex: 1 }}>
                    {p.label}
                    {current?.profileId === p.profileId ? "  ·  in use" : ""}
                  </Txt>
                  <HeaderButton
                    label="Edit"
                    onPress={() => setEditing({ draft: draftFromProfile(p), existing: p })}
                  />
                </View>
                <Txt variant="meta" tone="muted">
                  {providerChoice(p.kind).title} · {keyStatusText(status)}
                </Txt>
                <EgressBadge
                  text={egress.text}
                  local={egress.egress === "device" || egress.egress === "network"}
                />
              </View>
            );
          })}
          <View style={{ padding: t.space[2] }}>
            <Button title="Add a provider" onPress={() => setPicker(true)} />
          </View>
        </Section>
        {settings.profiles.length > 0 ? (
          <Section
            title="Per feature"
            footer="Send one feature to a different provider, for example dictation to OpenAI or a Whisper server on your network."
          >
            {FEATURE_ROWS.map((row) => {
              const sum = featureRouteSummary(settings, row.feature);
              return (
                <Row
                  key={row.feature}
                  title={row.label}
                  detail={sum.egress ? `${sum.provider} · ${sum.egress}` : sum.provider}
                  onPress={() => setRouting(row.feature)}
                  accessibilityHint="Choose the provider for this feature"
                />
              );
            })}
          </Section>
        ) : null}
        <View style={{ paddingHorizontal: t.space[2], marginTop: t.space[2], gap: t.space[1] }}>
          <Label>Where data goes</Label>
          <Txt variant="meta" tone="muted">
            Akatsuki AI processes requests without storing them. Your own key sends requests straight from
            this phone to that provider. An Ollama or OpenAI-compatible server on your network keeps mail on
            your network.
          </Txt>
        </View>
      </ScrollView>

      <ActionMenu
        visible={picker}
        onClose={() => setPicker(false)}
        title="Add a provider"
        items={PROVIDER_CHOICES.filter((c) => c.kind !== "akatsuki" || managedAiAllowed).map((c) => ({
          key: c.kind,
          label: c.title,
          onPress: () => setEditing({ draft: newDraft(newUuidV4(), c.kind as ProviderKind) }),
        }))}
      />
      <ActionMenu
        visible={routing !== null}
        onClose={() => setRouting(null)}
        title={FEATURE_ROWS.find((r) => r.feature === routing)?.label}
        items={
          routing
            ? [
                {
                  key: "default",
                  label: featureRoute(settings, routing) ? "Same as default" : "Same as default (in use)",
                  onPress: () => {
                    save({ routes: setFeatureRoute(settings, routing, null) }).catch((err) =>
                      showToast({ id: "ai-route", text: errorText(err), tone: "error" }),
                    );
                  },
                },
                ...routeChoices(settings, routing).map((p) => ({
                  key: p.profileId,
                  label:
                    featureRoute(settings, routing)?.profileId === p.profileId
                      ? `${p.label} (in use)`
                      : p.label,
                  onPress: () => {
                    save({ routes: setFeatureRoute(settings, routing, p.profileId) }).catch((err) =>
                      showToast({ id: "ai-route", text: errorText(err), tone: "error" }),
                    );
                  },
                })),
              ]
            : []
        }
      />
      {editing && deviceId ? (
        <ProfileEditor
          draft={editing.draft}
          existing={editing.existing}
          isDefault={current?.profileId === editing.draft.profileId}
          onClose={() => setEditing(null)}
          settings={settings}
          managedAiAllowed={managedAiAllowed}
          deviceId={deviceId}
        />
      ) : null}
    </Screen>
  );
}
