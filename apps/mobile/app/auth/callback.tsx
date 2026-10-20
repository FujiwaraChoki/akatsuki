// `akatsuki://auth/callback?code=…` from a magic link (§6.4, D-408). See `acceptHandoff` for the checks.
import { router, useLocalSearchParams } from "expo-router";
import { useEffect, useState } from "react";
import { View } from "react-native";
import { Orbs } from "../../src/components/Orbs";
import { Button, Screen, Spinner, Txt } from "../../src/components/ui";
import { acceptHandoff } from "../../src/lib/auth";
import { useTheme } from "../../src/theme/ThemeProvider";

export default function AuthCallback() {
  const t = useTheme();
  const params = useLocalSearchParams<{ code?: string; error?: string }>();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let done = false;
    acceptHandoff({ code: params.code, error: params.error }).then((r) => {
      if (done) return;
      if (r.ok) router.replace("/inbox");
      else setError(r.error);
    });
    return () => {
      done = true;
    };
  }, [params.code, params.error]);

  return (
    <Screen>
      <Orbs />
      <View
        style={{
          flex: 1,
          alignItems: "center",
          justifyContent: "center",
          padding: t.space[3],
          gap: t.space[2],
        }}
      >
        {error ? (
          <>
            <Txt variant="heading">Could not sign in</Txt>
            <Txt variant="reading" tone="secondary" style={{ textAlign: "center" }}>
              {error}
            </Txt>
            <Button title="Back to sign-in" variant="primary" onPress={() => router.replace("/sign-in")} />
          </>
        ) : (
          <>
            <Spinner />
            <Txt variant="body" tone="muted">
              Signing you in…
            </Txt>
          </>
        )}
      </View>
    </Screen>
  );
}
