// Sign-in (ARCHITECTURE §6.2): email code (OTP) first, magic link as the alternative, and Google / Microsoft
// buttons when the deployment has them (D-261). Better Auth runs on Convex; crossDomain brings social and
// magic-link sign-ins back here with `?ott=`, which ConvexBetterAuthProvider exchanges.
// First Light (SPEC §3.2): no card. One 360 column, the serif headline whose period is the dawn dot, the accent pill
// (one of its three uses), and the horizon low on the page with the licence line under it.
import { Button, errorMessage } from "@akatsuki/ui";
import { AUTH_COPY } from "@akatsuki/ui/copy";
import { type FormEvent, type ReactNode, useEffect, useId, useRef, useState } from "react";
import { useBackend } from "../backend/context.tsx";
import type { SocialProvider } from "../backend/types.ts";
import { codeOf } from "../components/ErrorState.tsx";

type Step = { kind: "email" } | { kind: "code"; email: string } | { kind: "link"; email: string };

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const PROVIDER_LABEL: Record<SocialProvider, string> = { google: "Google", microsoft: "Microsoft" };

/** Monochrome provider glyphs (SPEC §2.11): drawn in the text colour, never the brand colours. */
const PROVIDER_GLYPH: Record<SocialProvider, ReactNode> = {
  google: (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">
      <path
        d="M14.2 8.15c0-.5-.04-.98-.13-1.44H8v2.73h3.48a3 3 0 0 1-1.29 1.96v1.63h2.09c1.22-1.13 1.92-2.79 1.92-4.88Z"
        fill="currentColor"
      />
      <path
        d="M8 14.5c1.75 0 3.21-.58 4.28-1.57l-2.09-1.63c-.58.39-1.32.62-2.19.62-1.69 0-3.12-1.14-3.63-2.67H2.21v1.68A6.5 6.5 0 0 0 8 14.5Z"
        fill="currentColor"
        opacity="0.75"
      />
      <path
        d="M4.37 9.25a3.9 3.9 0 0 1 0-2.5V5.07H2.21a6.5 6.5 0 0 0 0 5.86l2.16-1.68Z"
        fill="currentColor"
        opacity="0.55"
      />
      <path
        d="M8 4.08c.95 0 1.8.33 2.47.97l1.85-1.85A6.5 6.5 0 0 0 2.21 5.07l2.16 1.68C4.88 5.22 6.31 4.08 8 4.08Z"
        fill="currentColor"
        opacity="0.85"
      />
    </svg>
  ),
  microsoft: (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">
      <path d="M2 2h5.5v5.5H2zM8.5 2H14v5.5H8.5zM2 8.5h5.5V14H2zM8.5 8.5H14V14H8.5z" fill="currentColor" />
    </svg>
  ),
};

export function SignIn() {
  const backend = useBackend();
  const [step, setStep] = useState<Step>({ kind: "email" });
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<null | "code" | "link" | "verify" | SocialProvider>(null);
  const [error, setError] = useState<string | null>(null);
  const emailId = useId();
  const codeId = useId();
  const errId = useId();
  const codeRef = useRef<HTMLInputElement>(null);
  const callbackURL = `${window.location.origin}/`;

  const run = async (what: typeof busy, fn: () => Promise<void>, invalid?: string) => {
    setBusy(what);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(invalid && codeOf(e) === "invalid_argument" ? invalid : errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const validEmail = EMAIL.test(email.trim());

  // Focus the code field once it is on screen (a microtask runs before React commits the step).
  const codeStepFor = step.kind === "code" ? step.email : null;
  useEffect(() => {
    if (codeStepFor !== null) codeRef.current?.focus();
  }, [codeStepFor]);

  const sendCode = (e?: FormEvent) => {
    e?.preventDefault();
    if (!validEmail) {
      setError("Enter a valid email address.");
      return;
    }
    const addr = email.trim();
    void run("code", async () => {
      await backend.auth.sendOtp(addr);
      setCode("");
      setStep({ kind: "code", email: addr });
    });
  };

  const sendLink = () => {
    if (!validEmail) {
      setError("Enter a valid email address.");
      return;
    }
    const addr = email.trim();
    void run("link", async () => {
      await backend.auth.sendMagicLink(addr, callbackURL);
      setStep({ kind: "link", email: addr });
    });
  };

  const verify = (e: FormEvent) => {
    e.preventDefault();
    if (step.kind !== "code") return;
    const otp = code.replace(/\s+/g, "");
    if (!/^\d{6}$/.test(otp)) {
      setError("Enter the 6-digit code from the email.");
      return;
    }
    void run(
      "verify",
      () => backend.auth.verifyOtp(step.email, otp),
      "That code is not right, or it has expired.",
    );
  };

  const digits = code.replace(/\D/g, "").slice(0, 6);
  const errorLine = error && (
    <p id={errId} className="ak-auth__error" role="alert">
      <span className="ak-auth__bang" aria-hidden="true">
        !
      </span>
      {error}
    </p>
  );

  return (
    <main className="ak-auth">
      <a className="ak-auth__dot" href="https://akatsuki.vc">
        <span className="ak-visually-hidden">Akatsuki</span>
      </a>
      <div className="ak-auth__column">
        {step.kind === "email" && (
          <form onSubmit={sendCode} noValidate aria-describedby={error ? errId : undefined}>
            <h1 className="ak-auth__headline">
              Mail at the speed of thought
              {/* The period is real text (so the heading reads "…thought."), drawn as the dawn dot. */}
              <span className="ak-auth__period">.</span>
            </h1>
            <p className="ak-auth__lead">One inbox for Gmail, Outlook and IMAP, driven from the keyboard.</p>
            <label htmlFor={emailId} className="ak-field__label">
              Email
            </label>
            <input
              id={emailId}
              className="ak-input"
              type="email"
              inputMode="email"
              autoComplete="email"
              // biome-ignore lint/a11y/noAutofocus: the only field on a sign-in page
              autoFocus
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              aria-invalid={error ? true : undefined}
            />
            {errorLine}
            <div className="ak-auth__actions">
              <Button type="submit" variant="brand" size="lg" loading={busy === "code"}>
                {AUTH_COPY.sendCode}
              </Button>
              <button
                type="button"
                className="ak-auth__text"
                onClick={sendLink}
                disabled={busy === "link"}
                aria-busy={busy === "link" || undefined}
              >
                {AUTH_COPY.sendLink}
              </button>
            </div>
            {backend.loginProviders.length > 0 && (
              <>
                <div className="ak-auth__divider">
                  <span>or</span>
                </div>
                <div className="ak-auth__social">
                  {backend.loginProviders.map((p) => (
                    <Button
                      key={p}
                      size="lg"
                      variant="secondary"
                      loading={busy === p}
                      icon={PROVIDER_GLYPH[p]}
                      onClick={() => run(p, () => backend.auth.signInSocial(p, callbackURL))}
                    >
                      Continue with {PROVIDER_LABEL[p]}
                    </Button>
                  ))}
                </div>
              </>
            )}
            {backend.kind === "mock" && <p className="ak-auth__demo">Demo mode: any address, code 123456.</p>}
          </form>
        )}
        {step.kind === "code" && (
          <form onSubmit={verify} noValidate>
            <h1 className="ak-auth__headline ak-auth__headline--step">{AUTH_COPY.checkEmail}</h1>
            <p className="ak-auth__note">
              We sent a 6-digit code to <strong>{step.email}</strong> ·{" "}
              <button type="button" className="ak-textbutton" onClick={() => setStep({ kind: "email" })}>
                Change
              </button>
            </p>
            <label htmlFor={codeId} className="ak-visually-hidden">
              Code
            </label>
            <div className="ak-code" data-invalid={error ? "" : undefined}>
              <input
                id={codeId}
                ref={codeRef}
                className="ak-code__input"
                inputMode="numeric"
                autoComplete="one-time-code"
                maxLength={7}
                value={code}
                onChange={(e) => setCode(e.target.value.replace(/[^\d ]/g, ""))}
                aria-invalid={error ? true : undefined}
              />
              {Array.from({ length: 6 }, (_, i) => (
                <span
                  // biome-ignore lint/suspicious/noArrayIndexKey: six fixed cells
                  key={i}
                  className="ak-code__cell"
                  data-current={i === Math.min(digits.length, 5) || undefined}
                  data-filled={digits[i] !== undefined || undefined}
                  aria-hidden="true"
                >
                  {digits[i] ?? ""}
                </span>
              ))}
            </div>
            {errorLine}
            <div className="ak-auth__actions">
              <Button type="submit" variant="brand" size="lg" loading={busy === "verify"}>
                {AUTH_COPY.verify}
              </Button>
              <button
                type="button"
                className="ak-auth__text"
                onClick={() => sendCode()}
                disabled={busy === "code"}
              >
                {AUTH_COPY.resendCode}
              </button>
            </div>
          </form>
        )}
        {step.kind === "link" && (
          <div>
            <h1 className="ak-auth__headline ak-auth__headline--step">{AUTH_COPY.checkEmail}</h1>
            <p className="ak-auth__note">
              Open the link we sent to <strong>{step.email}</strong> in this browser to finish signing in.
            </p>
            {errorLine}
            <div className="ak-auth__actions">
              <button type="button" className="ak-auth__text" onClick={() => setStep({ kind: "email" })}>
                {AUTH_COPY.otherEmail}
              </button>
            </div>
          </div>
        )}
      </div>
      <div className="ak-horizon ak-auth__horizon" aria-hidden="true">
        <span className="ak-horizon__band" />
        <span className="ak-horizon__line" />
      </div>
      <p className="ak-auth__footer">Open source, free for personal use.</p>
    </main>
  );
}
