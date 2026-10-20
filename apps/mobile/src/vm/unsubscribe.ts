// Where a mailto unsubscribe goes, before anything is sent (the same rule as the web, D-397 follow-up): the
// address, and whether it is at the sender's domain. The worker sends a fixed or token-only subject.

/** The address of a `mailto:` URL, or null. */
export function mailtoAddress(mailto: string): string | null {
  const m = /^mailto:([^?]*)/i.exec(mailto.trim());
  if (!m) return null;
  let addr: string;
  try {
    addr = decodeURIComponent(m[1]!).split(",")[0]!.trim().toLowerCase();
  } catch {
    return null;
  }
  return /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(addr) ? addr : null;
}

const baseDomain = (d: string) => d.toLowerCase().replace(/\.+$/, "").split(".").slice(-2).join(".");

/** The confirmation text, or null when the header has no usable address. */
export function mailtoConfirmation(
  mailto: string,
  senderEmail: string | undefined,
): { to: string; foreign: boolean; message: string } | null {
  const to = mailtoAddress(mailto);
  if (!to) return null;
  const senderDomain = senderEmail?.split("@")[1];
  const foreign = senderDomain !== undefined && baseDomain(senderDomain) !== baseDomain(to.split("@")[1]!);
  return {
    to,
    foreign,
    message: foreign
      ? `Akatsuki would email ${to} from your account to ask to be removed. That address is not at the sender's domain: only continue if you trust this message.`
      : `Akatsuki emails ${to} from your account to ask to be removed.`,
  };
}
