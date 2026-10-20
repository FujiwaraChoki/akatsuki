// Sign-in (§6.4): magic link to `akatsuki://auth/callback`, email OTP, and Google / Microsoft when the server has
// them configured (EXPO_PUBLIC_LOGIN_PROVIDERS, D-332). Login identity only: mailboxes are connected separately.
import { AUTH_COPY } from "@akatsuki/ui/copy";
import { Redirect, router } from "expo-router";
import { useEffect, useState } from "react";
import { KeyboardAvoidingView, Linking, Platform, Pressable, ScrollView, View } from "react-native";
import { Orbs } from "../src/components/Orbs";
import { Button, Field, Label, Screen, Txt } from "../src/components/ui";
import { ENV } from "../src/env";
import { authClient, sendMagicLink, sendOtp, signInSocial, verifyOtp } from "../src/lib/auth";
import { useTheme } from "../src/theme/ThemeProvider";
import { isEmail } from "../src/vm/compose";

type Step = { kind: "email" } | { kind: "link_sent" } | { kind: "otp" };

const RESEND_SECONDS = 30;
const PROVIDER_TITLES = { google: "Continue with Google", microsoft: "Continue with Microsoft" } as const;

export default function SignIn() {
  const t = useTheme();
  const session = authClient!.useSession();
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [step, setStep] = useState<Step>({ kind: "email" });
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cooldown, setCooldown] = useState(0);

  useEffect(() => {
    if (cooldown <= 0) return;
    const id = setTimeout(() => setCooldown((c) => c - 1), 1000);
    return () => clearTimeout(id);
  }, [cooldown]);

  if (session.data) return <Redirect href="/inbox" />;

  const address = email.trim().toLowerCase();
  const valid = isEmail(address);

  async function link() {
    if (!valid) return setError("Enter your email address.");
    setBusy("link");
    setError(null);
    const r = await sendMagicLink(address);
    setBusy(null);
    if (!r.ok) return setError(r.error);
    setCooldown(RESEND_SECONDS);
    setStep({ kind: "link_sent" });
  }

  async function otp() {
    if (!valid) return setError("Enter your email address.");
    setBusy("otp");
    setError(null);
    const r = await sendOtp(address);
    setBusy(null);
    if (!r.ok) return setError(r.error);
    setCooldown(RESEND_SECONDS);
    setStep({ kind: "otp" });
  }

  async function verify() {
    const c = code.replace(/\s/g, "");
    if (!/^\d{4,10}$/.test(c)) return setError("Enter the code from the email.");
    setBusy("verify");
    setError(null);
    const r = await verifyOtp(address, c);
    setBusy(null);
    if (!r.ok) return setError(r.error);
    router.replace("/inbox");
  }

  async function social(provider: "google" | "microsoft") {
    setBusy(provider);
    setError(null);
    const r = await signInSocial(provider);
    setBusy(null);
    if (!r.ok) setError(r.error);
  }

  return (
    <Screen edges={["top", "bottom", "left", "right"]}>
      <Orbs />
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={{ flex: 1 }}>
        <ScrollView
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={{
            flexGrow: 1,
            justifyContent: "center",
            padding: t.space[3],
            gap: t.space[3],
            maxWidth: 480,
            width: "100%",
            alignSelf: "center",
          }}
        >
          <View style={{ gap: t.space[1] }}>
            <Txt
              accessibilityRole="header"
              style={{
                fontFamily: t.text.heading.fontFamily,
                fontSize: 56,
                lineHeight: 60,
                letterSpacing: -1,
                color: t.colors.text,
              }}
            >
              Akatsuki
            </Txt>
            <Label>Fast, keyboard-first email</Label>
          </View>

          {step.kind === "email" ? (
            <View style={{ gap: t.space[2] }}>
              <Field
                label="Email"
                value={email}
                onChangeText={(v) => {
                  setEmail(v);
                  setError(null);
                }}
                placeholder="you@example.com"
                autoCapitalize="none"
                autoCorrect={false}
                autoComplete="email"
                keyboardType="email-address"
                textContentType="emailAddress"
                returnKeyType="send"
                onSubmitEditing={otp}
                error={error}
                autoFocus
              />
              {/* The email code is the primary method on every client (web, desktop, mobile; D-399). */}
              <Button
                title={AUTH_COPY.sendCode}
                variant="primary"
                onPress={otp}
                busy={busy === "otp"}
                disabled={!valid}
              />
              <Button
                title={AUTH_COPY.sendLink}
                variant="ghost"
                onPress={link}
                busy={busy === "link"}
                disabled={!valid}
              />
              {(ENV.env?.loginProviders ?? []).length > 0 ? (
                <View style={{ gap: t.space[1], marginTop: t.space[1] }}>
                  <Label>Or</Label>
                  {ENV.env!.loginProviders.map((p) => (
                    <Button key={p} title={PROVIDER_TITLES[p]} onPress={() => social(p)} busy={busy === p} />
                  ))}
                </View>
              ) : null}
            </View>
          ) : null}

          {step.kind === "link_sent" ? (
            <View style={{ gap: t.space[2] }} accessibilityLiveRegion="polite">
              <Txt variant="heading">{AUTH_COPY.checkEmail}</Txt>
              <Txt variant="reading" tone="secondary">
                We sent a sign-in link to {address}. Open it on this device within 15 minutes.
              </Txt>
              {error ? (
                <Txt variant="meta" tone="error">
                  {error}
                </Txt>
              ) : null}
              <Button
                title={cooldown > 0 ? `Send again in ${cooldown} s` : "Send again"}
                onPress={link}
                disabled={cooldown > 0}
                busy={busy === "link"}
              />
              <Button
                title={AUTH_COPY.useCodeInstead}
                variant="ghost"
                onPress={otp}
                busy={busy === "otp"}
                disabled={cooldown > 0}
              />
              <Button
                title={AUTH_COPY.otherEmail}
                variant="ghost"
                onPress={() => setStep({ kind: "email" })}
              />
            </View>
          ) : null}

          {step.kind === "otp" ? (
            <View style={{ gap: t.space[2] }}>
              <Txt variant="reading" tone="secondary">
                Enter the code we sent to {address}.
              </Txt>
              <Field
                label="Code"
                value={code}
                onChangeText={(v) => {
                  setCode(v);
                  setError(null);
                }}
                keyboardType="number-pad"
                textContentType="oneTimeCode"
                autoComplete="one-time-code"
                maxLength={10}
                returnKeyType="done"
                onSubmitEditing={verify}
                error={error}
                autoFocus
              />
              <Button title={AUTH_COPY.verify} variant="primary" onPress={verify} busy={busy === "verify"} />
              <Button
                title={cooldown > 0 ? `Send a new code in ${cooldown} s` : AUTH_COPY.resendCode}
                variant="ghost"
                onPress={otp}
                disabled={cooldown > 0}
              />
              <Button
                title={AUTH_COPY.otherEmail}
                variant="ghost"
                onPress={() => setStep({ kind: "email" })}
              />
            </View>
          ) : null}

          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: t.space[2] }}>
            <Pressable accessibilityRole="link" onPress={() => Linking.openURL("https://akatsuki.vc/terms")}>
              <Txt variant="meta" tone="muted">
                Terms
              </Txt>
            </Pressable>
            <Pressable
              accessibilityRole="link"
              onPress={() => Linking.openURL("https://akatsuki.vc/privacy")}
            >
              <Txt variant="meta" tone="muted">
                Privacy
              </Txt>
            </Pressable>
          </View>
        </ScrollView>
      </KeyboardAvoidingView>
    </Screen>
  );
}
