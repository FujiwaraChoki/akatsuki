// The "where your data goes" label under every AI action (ARCHITECTURE §12.2, design spec §3.8, §4.9): a mono
// label in `muted`, no fill, from packages/ai `egressText`. `full` adds the whole sentence (where the user decides to
// send something: the dictation and write sheets, settings).
import { View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { egressShort } from "../vm/egress";
import { Crumb, Txt } from "./ui";

export function EgressLine({ text, full = false, what }: { text: string; full?: boolean; what?: string }) {
  const t = useTheme();
  return (
    <View accessible accessibilityLabel={`Where your data goes: ${text}`} style={{ gap: 2 }}>
      <Crumb>{what ? `${what} · ${egressShort(text)}` : egressShort(text)}</Crumb>
      {full ? (
        <Txt variant="meta" tone="tertiary" style={{ marginTop: t.space["0_5"] }}>
          {text}.
        </Txt>
      ) : null}
    </View>
  );
}
