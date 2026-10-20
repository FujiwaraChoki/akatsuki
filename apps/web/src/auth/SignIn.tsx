// Sign-in (ARCHITECTURE §6.2): email code (OTP) first, magic link as the alternative, and Google / Microsoft
// buttons when the deployment has them (D-261). Better Auth runs on Convex; crossDomain brings social and
// magic-link sign-ins back here with `?ott=`, which ConvexBetterAuthProvider exchanges.
import { Button, Eyebrow, errorMessage, Heading, Orbs } from "@akatsuki/ui";
import { AUTH_COPY } from "@akatsuki/ui/copy";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { useBackend } from "../backend/context.tsx";
import type { SocialProvider } from "../backend/types.ts";
import { codeOf } from "../components/ErrorState.tsx";

type Step = { kind: "email" } | { kind: "code"; email: string } | { kind: "link"; email: string };

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const PROVIDER_LABEL: Record<SocialProvider, string> = { google: "Google", microsoft: "Microsoft" };

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

  return (
    <main className="ak-auth">
      <Orbs variant="hero" />
      <div className="ak-auth__card">
        <p className="ak-auth__brand">
          <span className="ak-auth__mark" aria-hidden="true" />
          Akatsuki
        </p>
        {step.kind === "email" && (
          <form onSubmit={sendCode} noValidate aria-describedby={error ? errId : undefined}>
            <Eyebrow>Sign in</Eyebrow>
            <Heading level={1} variant="heading" className="ak-auth__title">
              Mail at the speed of thought.
            </Heading>
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
            <div className="ak-auth__actions">
              <Button type="submit" variant="primary" size="lg" loading={busy === "code"}>
                {AUTH_COPY.sendCode}
              </Button>
              <Button size="lg" variant="ghost" onClick={sendLink} loading={busy === "link"}>
                {AUTH_COPY.sendLink}
              </Button>
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
                      onClick={() => run(p, () => backend.auth.signInSocial(p, callbackURL))}
                    >
                      Continue with {PROVIDER_LABEL[p]}
                    </Button>
                  ))}
                </div>
              </>
            )}
          </form>
        )}
        {step.kind === "code" && (
          <form onSubmit={verify} noValidate>
            <Eyebrow>Check your email</Eyebrow>
            <Heading level={1} variant="heading" className="ak-auth__title">
              Enter your code.
            </Heading>
            <p className="ak-auth__note">
              We sent a 6-digit code to <strong>{step.email}</strong>. It expires in a few minutes.
            </p>
            <label htmlFor={codeId} className="ak-field__label">
              Code
            </label>
            <input
              id={codeId}
              ref={codeRef}
              className="ak-input ak-input--code"
              inputMode="numeric"
              autoComplete="one-time-code"
              maxLength={7}
              value={code}
              onChange={(e) => setCode(e.target.value.replace(/[^\d ]/g, ""))}
              placeholder="000000"
              aria-invalid={error ? true : undefined}
            />
            <div className="ak-auth__actions">
              <Button type="submit" variant="primary" size="lg" loading={busy === "verify"}>
                {AUTH_COPY.verify}
              </Button>
              <Button variant="ghost" onClick={() => sendCode()} loading={busy === "code"}>
                {AUTH_COPY.resendCode}
              </Button>
              <Button variant="ghost" onClick={() => setStep({ kind: "email" })}>
                {AUTH_COPY.otherEmail}
              </Button>
            </div>
          </form>
        )}
        {step.kind === "link" && (
          <div>
            <Eyebrow>Check your email</Eyebrow>
            <Heading level={1} variant="heading" className="ak-auth__title">
              Your link is on its way.
            </Heading>
            <p className="ak-auth__note">
              Open the link we sent to <strong>{step.email}</strong> in this browser to finish signing in.
            </p>
            <div className="ak-auth__actions">
              <Button variant="ghost" onClick={() => setStep({ kind: "email" })}>
                {AUTH_COPY.otherEmail}
              </Button>
            </div>
          </div>
        )}
        {error && (
          <p id={errId} className="ak-auth__error" role="alert">
            {error}
          </p>
        )}
        {backend.kind === "mock" && <p className="ak-auth__demo">Demo mode: any address, code 123456.</p>}
      </div>
      <p className="ak-auth__footer">Open source, free for personal use.</p>
    </main>
  );
}
