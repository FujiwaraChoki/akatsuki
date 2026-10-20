// Settings: Mail (undo send, remote images, read receipts, device cache, unified inbox), Appearance, Shortcuts
// (Superhuman or Gmail keymap), Notifications. Every change is one `me.updatePrefs` patch (§4.2).
import { Button, errorMessage, useToast } from "@akatsuki/ui";
import { useState } from "react";
import { useBackend } from "../../backend/context.tsx";
import type { MeView, Prefs } from "../../backend/types.ts";
import { setAppearance } from "../../lib/theme.ts";
import { ShortcutGroups } from "../../mail/HelpDialog.tsx";
import { Row, Section, Segmented, Switch } from "../ui.tsx";

function usePrefs(me: MeView) {
  const backend = useBackend();
  const m = backend.useMutations();
  const { toast } = useToast();
  const fail = (e: unknown) => toast({ title: errorMessage(e), tone: "error" });
  const set = (patch: Partial<Prefs>) => void m.updatePrefs({ patch }).catch(fail);
  /** Theme and density apply at once and survive a reload before the save lands (lib/theme.ts). */
  const setLook = (patch: Pick<Partial<Prefs>, "theme" | "density">) =>
    void setAppearance(patch, (p) => m.updatePrefs({ patch: p })).catch(fail);
  return { prefs: me.profile.prefs, set, setLook };
}

export const READ_RECEIPT_WARNING =
  "Read receipts are unreliable. Akatsuki adds a tiny image to the messages you choose; many mail apps block " +
  "it, and Apple Mail Privacy Protection loads every image in advance, so a message can look opened when nobody " +
  "read it, or never look opened at all. Recipients are not told. Treat an open as a hint, never as proof.";

export function MailSection({ me }: { me: MeView }) {
  const { prefs, set } = usePrefs(me);
  const [confirming, setConfirming] = useState(false);
  const [ack, setAck] = useState(false);
  return (
    <>
      <Section title="Sending" description="Every message waits a moment before it leaves, so Undo works.">
        <Row label="Undo send" hint="How long Undo stays available after you press Send.">
          <Segmented
            label="Undo send window"
            value={prefs.undoSendSeconds}
            options={[0, 5, 10, 20, 30].map((v) => ({ value: v, label: v === 0 ? "Off" : `${v} s` }))}
            onChange={(v) => set({ undoSendSeconds: v })}
          />
        </Row>
        <Row
          label="Read receipts"
          hint={
            prefs.readReceipts.enabled
              ? "On. The composer offers a receipt per message; it is never added on its own."
              : "Off. Nothing in your messages reports when they are opened."
          }
        >
          <Switch
            label="Read receipts"
            checked={prefs.readReceipts.enabled}
            onChange={(v) => {
              if (!v) return set({ readReceipts: { ...prefs.readReceipts, enabled: false } });
              setAck(false);
              setConfirming(true);
            }}
          />
        </Row>
        {confirming && (
          <div
            className="ak-settings__warning"
            role="alertdialog"
            aria-label="Before you turn on read receipts"
          >
            <p>{READ_RECEIPT_WARNING}</p>
            <label className="ak-check">
              <input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} />I understand
              that read receipts are unreliable.
            </label>
            <div className="ak-settings__actions">
              <Button
                variant="primary"
                disabled={!ack}
                onClick={() => {
                  set({ readReceipts: { enabled: true, warningAcknowledgedAt: Date.now() } });
                  setConfirming(false);
                }}
              >
                Turn on read receipts
              </Button>
              <Button variant="ghost" onClick={() => setConfirming(false)}>
                Keep them off
              </Button>
            </div>
          </div>
        )}
      </Section>
      <Section title="Reading">
        <Row
          label="Remote images"
          hint="Blocked by default so senders cannot tell when you read. Allowed images load through Akatsuki's proxy, never from your browser."
        >
          <Segmented
            label="Remote images"
            value={prefs.remoteImages}
            options={[
              { value: "block", label: "Ask first" },
              { value: "proxy", label: "Always load" },
            ]}
            onChange={(v) => set({ remoteImages: v })}
          />
        </Row>
        <Row
          label="Keep messages on this device"
          hint="Opened messages are cached in this browser (up to 250 MB) so they open instantly. Off: memory only."
        >
          <Switch
            label="Keep messages on this device"
            checked={prefs.cacheBodiesOnDevice}
            onChange={(v) => set({ cacheBodiesOnDevice: v })}
          />
        </Row>
        <Row label="Unified inbox" hint="Show every connected mailbox in one inbox.">
          <Switch
            label="Unified inbox"
            checked={prefs.unifiedInbox}
            onChange={(v) => set({ unifiedInbox: v })}
          />
        </Row>
      </Section>
    </>
  );
}

export function AppearanceSection({ me }: { me: MeView }) {
  const { prefs, setLook } = usePrefs(me);
  return (
    <Section
      title="Appearance"
      description="Crimson dawn, light or dark. Motion follows your system setting."
    >
      <Row label="Theme">
        <Segmented
          label="Theme"
          value={prefs.theme}
          options={[
            { value: "system", label: "System" },
            { value: "light", label: "Light" },
            { value: "dark", label: "Dark" },
          ]}
          onChange={(v) => setLook({ theme: v })}
        />
      </Row>
      <Row label="Density" hint="Row height in conversation lists.">
        <Segmented
          label="Density"
          value={prefs.density}
          options={[
            { value: "comfortable", label: "Comfortable" },
            { value: "compact", label: "Compact" },
          ]}
          onChange={(v) => setLook({ density: v })}
        />
      </Row>
    </Section>
  );
}

export function ShortcutsSection({ me }: { me: MeView }) {
  const { prefs, set } = usePrefs(me);
  return (
    <Section
      title="Shortcuts"
      description="Every action has a key and a command palette entry (Ctrl or ⌘ K). Pick the keymap your fingers know."
    >
      <Row
        label="Keymap"
        hint="Gmail keys change a few bindings, for example b to snooze and Shift I to mark read."
      >
        <Segmented
          label="Keymap"
          value={prefs.keymap}
          options={[
            { value: "superhuman", label: "Superhuman" },
            { value: "gmail", label: "Gmail" },
          ]}
          onChange={(v) => set({ keymap: v })}
        />
      </Row>
      <ShortcutGroups />
    </Section>
  );
}

export function NotificationsSection({ me }: { me: MeView }) {
  const { prefs, set } = usePrefs(me);
  return (
    <Section
      title="Notifications"
      description="Push notifications go to the Akatsuki mobile app through Expo, Apple and Google."
    >
      <Row
        label="New mail"
        hint={
          prefs.notifications === "full"
            ? "Shows the sender and subject. Expo, Apple and Google see that text, and so does your lock screen."
            : prefs.notifications === "private"
              ? "Shows only a count, such as “3 new messages”. No sender or subject leaves Akatsuki."
              : "No notifications."
        }
      >
        <Segmented
          label="New mail notifications"
          value={prefs.notifications}
          options={[
            { value: "off", label: "Off" },
            { value: "private", label: "Count only" },
            { value: "full", label: "Sender and subject" },
          ]}
          onChange={(v) => set({ notifications: v })}
        />
      </Row>
    </Section>
  );
}
