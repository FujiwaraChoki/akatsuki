// Status banners above the list: offline, accounts that need a reconnect, accounts still syncing (§15).
import { Button, maskAccount } from "@akatsuki/ui";
import { useEffect, useState } from "react";
import { useBackend } from "../backend/context.tsx";
import type { MailAccountView } from "../backend/types.ts";

function useOnline(): boolean {
  const [online, setOnline] = useState(() => (typeof navigator === "undefined" ? true : navigator.onLine));
  useEffect(() => {
    const on = () => setOnline(true);
    const off = () => setOnline(false);
    window.addEventListener("online", on);
    window.addEventListener("offline", off);
    return () => {
      window.removeEventListener("online", on);
      window.removeEventListener("offline", off);
    };
  }, []);
  return online;
}

/** True after the live connection has been down for `graceMs` (short blips are not worth a banner). */
function useOffline(graceMs = 2500): boolean {
  const backend = useBackend();
  const connected = backend.useConnected();
  const online = useOnline();
  const down = !connected || !online;
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!down) {
      setShown(false);
      return;
    }
    const t = setTimeout(() => setShown(true), graceMs);
    return () => clearTimeout(t);
  }, [down, graceMs]);
  return shown;
}

export function Banners({
  accounts,
  onReconnect,
}: {
  accounts: readonly MailAccountView[];
  onReconnect: (a: MailAccountView) => void;
}) {
  const offline = useOffline();
  const reauth = accounts.filter((a) => a.status === "reauth_required");
  const syncing = accounts.filter((a) => a.status === "initial_sync" || a.status === "connecting");
  const degraded = accounts.filter((a) => a.status === "degraded");
  if (!offline && reauth.length === 0 && syncing.length === 0 && degraded.length === 0) return null;
  return (
    <div className="ak-banners">
      {offline && (
        <p className="ak-banner ak-banner--warn" role="status">
          You are offline. Changes are saved on this device and sent when the connection is back.
        </p>
      )}
      {reauth.map((a) => (
        <div key={a._id} className="ak-banner ak-banner--error" role="alert">
          <span>{maskAccount(a.email)} needs to be reconnected before it can sync again.</span>
          <Button size="sm" variant="primary" onClick={() => onReconnect(a)}>
            Reconnect
          </Button>
        </div>
      ))}
      {degraded.map((a) => (
        <p key={a._id} className="ak-banner" role="status">
          Sync for {maskAccount(a.email)} is delayed. New mail may take a little longer.
        </p>
      ))}
      {syncing.map((a) => (
        <p key={a._id} className="ak-banner" role="status">
          Syncing {maskAccount(a.email)}
          {a.progress.threadsDone > 0 &&
            `: ${a.progress.threadsDone.toLocaleString()}${a.progress.threadsEstimated ? ` of about ${a.progress.threadsEstimated.toLocaleString()}` : ""} conversations`}
          .
        </p>
      ))}
    </div>
  );
}
