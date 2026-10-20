// Shown instead of the app when the device's secure storage cannot be used (lib/secure.ts records the first
// failure). Without it the session can never load, and the app would sit on a spinner.
import { View } from "react-native";
import { resetSecureFailure } from "../lib/secure";
import { useTheme } from "../theme/ThemeProvider";
import type { StorageFailure } from "../vm/storage";
import { Button, Txt } from "./ui";

export function StorageFailureScreen({ failure }: { failure: StorageFailure }) {
  const t = useTheme();
  return (
    <View
      accessibilityRole="alert"
      style={{
        flex: 1,
        backgroundColor: t.colors.background,
        padding: t.space[3],
        justifyContent: "center",
        gap: t.space[2],
      }}
    >
      <Txt variant="heading">{failure.title}</Txt>
      <Txt variant="body" tone="secondary">
        {failure.body}
      </Txt>
      {failure.detail ? (
        <Txt variant="code" tone="muted" selectable>
          {failure.detail}
        </Txt>
      ) : null}
      <Button title="Try again" variant="primary" onPress={resetSecureFailure} />
    </View>
  );
}
