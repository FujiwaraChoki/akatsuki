// The writing-style profile for "write in my voice" (§12.1 `voice_profile`): kept on this device, and in Convex
// `aiSettings.voiceProfile` only when the user opted in to syncing it (`consent.syncVoiceProfile`).
const KEY = "ak:voice-profile";

export function localVoice(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

export function setLocalVoice(profile: string | null): void {
  try {
    if (profile) localStorage.setItem(KEY, profile.slice(0, 2048));
    else localStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
}

/** The synced profile when there is one, else this device's. */
export function voiceFor(settings: { voiceProfile?: string }): string | null {
  return settings.voiceProfile ?? localVoice();
}
