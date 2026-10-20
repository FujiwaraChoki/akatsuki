// Keyboard shortcuts available in the mobile app, from the shared key map.
import { formatKeys, keysFor, SHORTCUTS } from "@akatsuki/shortcuts";
import { router } from "expo-router";
import { SectionList, View } from "react-native";
import { Header } from "../../../src/components/Header";
import { Label, Screen, Txt } from "../../../src/components/ui";
import { useKeyboard, useShortcuts } from "../../../src/state/keyboard";
import { useTheme } from "../../../src/theme/ThemeProvider";
import { MOBILE_SHORTCUTS } from "../../../src/vm/keyboard";

export default function Shortcuts() {
  const t = useTheme();
  const keyboard = useKeyboard();
  useShortcuts("list", { "nav.back": () => router.back() });
  const groups = new Map<string, { id: string; label: string; keys: string }[]>();
  for (const s of SHORTCUTS) {
    if (!MOBILE_SHORTCUTS.has(s.id)) continue;
    const keys = keysFor(s, keyboard.platform, keyboard.keymap)
      .map((k) => formatKeys(k, keyboard.platform))
      .join("  or  ");
    if (!keys) continue;
    groups.set(s.group, [...(groups.get(s.group) ?? []), { id: s.id, label: s.label, keys }]);
  }
  const sections = [...groups].map(([title, data]) => ({ title, data }));
  return (
    <Screen>
      <Header title="Shortcuts" onBack={() => router.back()} />
      <SectionList
        sections={sections}
        keyExtractor={(i) => i.id}
        renderSectionHeader={({ section }) => (
          <Label
            style={{
              paddingHorizontal: t.space[2],
              paddingTop: t.space[3],
              paddingBottom: t.space[1],
              backgroundColor: t.colors.background,
            }}
          >
            {section.title}
          </Label>
        )}
        renderItem={({ item }) => (
          <View
            style={{
              flexDirection: "row",
              paddingHorizontal: t.space[2],
              paddingVertical: t.space[1],
              gap: t.space[2],
            }}
          >
            <Txt variant="body" style={{ flex: 1 }}>
              {item.label}
            </Txt>
            <Txt variant="code" tone="secondary">
              {item.keys}
            </Txt>
          </View>
        )}
        contentContainerStyle={{ paddingBottom: t.space[8] }}
      />
    </Screen>
  );
}
