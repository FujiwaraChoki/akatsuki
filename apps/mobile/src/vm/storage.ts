// When the device's secure storage cannot be used (iOS Keychain / Android Keystore). A simulator build without a
// keychain entitlement throws on every SecureStore call; a locked or corrupted keystore can do the same on a
// phone. The app then shows this explanation instead of waiting on a session that can never load. Pure copy.

export interface StorageFailure {
  title: string;
  body: string;
  /** The platform's own message, shortened, for a bug report. */
  detail: string | null;
}

/** A short, single-line version of whatever the native module threw. */
function detailOf(err: unknown): string | null {
  const raw =
    err instanceof Error ? err.message : typeof err === "string" ? err : err == null ? "" : String(err);
  const line = raw.replace(/\s+/g, " ").trim();
  if (!line) return null;
  return line.length > 160 ? `${line.slice(0, 157)}…` : line;
}

export function storageFailure(err: unknown): StorageFailure {
  return {
    title: "Secure storage is unavailable",
    body:
      "Akatsuki keeps your sign-in and any API keys in this device's secure storage, and it could not be " +
      "opened. Close and reopen the app. If it keeps happening, reinstall the app. (A development build " +
      "needs a keychain entitlement: sign simulator builds ad hoc.)",
    detail: detailOf(err),
  };
}

/** The error thrown to callers of the async helpers, with the same plain sentence. */
export const SECURE_STORAGE_ERROR = "This device's secure storage is unavailable.";
