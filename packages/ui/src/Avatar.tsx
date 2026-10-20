// Avatars: an image when one is given (proxied by the caller; never a raw remote URL from mail), otherwise
// initials as a mono letter tile (First Light: no tinted circles; the sender's name carries the identity, so the
// mail views do not use avatars at all). `tintOf` stays as a stable hash for callers that group by sender.
import { type HTMLAttributes, useState } from "react";
import { cx } from "./config.tsx";

export interface AvatarProps extends HTMLAttributes<HTMLSpanElement> {
  name?: string;
  email: string;
  src?: string;
  size?: "sm" | "md" | "lg";
}

/** Up to two initials from a display name, else from the address's local part. */
export function initialsOf(name: string | undefined, email: string): string {
  const source = name?.trim() || email.split("@")[0] || "?";
  const words = source
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .split(/\s+/)
    .filter(Boolean);
  const letters = words.length >= 2 ? [words[0]!, words[words.length - 1]!] : [words[0] ?? "?"];
  return letters
    .map((w) => Array.from(w)[0] ?? "")
    .join("")
    .slice(0, 4)
    .toLocaleUpperCase();
}

/** A stable 0..2 tint step from an address (FNV-1a). */
export function tintOf(email: string): 0 | 1 | 2 {
  let h = 0x811c9dc5;
  for (const ch of email.toLowerCase()) {
    h ^= ch.codePointAt(0)!;
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return (h % 3) as 0 | 1 | 2;
}

export function Avatar({ name, email, src, size = "md", className, ...rest }: AvatarProps) {
  const [failed, setFailed] = useState(false);
  const label = name?.trim() || email;
  return (
    <span
      role="img"
      aria-label={label}
      className={cx("ak-avatar", `ak-avatar--${size}`, `ak-avatar--tint-${tintOf(email)}`, className)}
      {...rest}
    >
      {src && !failed ? (
        <img className="ak-avatar__img" src={src} alt="" onError={() => setFailed(true)} />
      ) : (
        <span className="ak-avatar__initials" aria-hidden="true">
          {initialsOf(name, email)}
        </span>
      )}
    </span>
  );
}
