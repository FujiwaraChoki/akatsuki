// A bottom sheet of actions (the thread's "More" menu). Each item shows its keyboard hint on devices with a
// keyboard, so the menu also teaches the shortcuts.
import { Modal, Pressable, ScrollView, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "../theme/ThemeProvider";
import { Txt } from "./ui";

export interface MenuItem {
  key: string;
  label: string;
  hint?: string | null;
  destructive?: boolean;
  onPress: () => void;
}

export function ActionMenu({
  visible,
  onClose,
  items,
  title,
}: {
  visible: boolean;
  onClose: () => void;
  items: MenuItem[];
  title?: string;
}) {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose} statusBarTranslucent>
      <Pressable
        accessibilityLabel="Close menu"
        onPress={onClose}
        style={{ flex: 1, backgroundColor: t.colors.scrim }}
      />
      <View
        accessibilityViewIsModal
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          maxHeight: "75%",
          backgroundColor: t.colors.background,
          borderTopLeftRadius: t.radius.card,
          borderTopRightRadius: t.radius.card,
          paddingBottom: insets.bottom + t.space[1],
          paddingTop: t.space[1],
          borderTopWidth: 1,
          borderColor: t.colors.borderSubtle,
        }}
      >
        {title ? (
          <Txt
            variant="label"
            tone="muted"
            style={{ paddingHorizontal: t.space[3], paddingVertical: t.space[1] }}
          >
            {title}
          </Txt>
        ) : null}
        <ScrollView>
          {items.map((item) => (
            <Pressable
              key={item.key}
              accessibilityRole="button"
              onPress={() => {
                onClose();
                item.onPress();
              }}
              style={({ pressed }) => ({
                flexDirection: "row",
                alignItems: "center",
                minHeight: 52,
                paddingHorizontal: t.space[3],
                backgroundColor: pressed ? t.colors.selected : "transparent",
              })}
            >
              <Txt variant="body" tone={item.destructive ? "accent" : "text"} style={{ flex: 1 }}>
                {item.label}
              </Txt>
              {item.hint ? (
                <Txt variant="code" tone="muted">
                  {item.hint}
                </Txt>
              ) : null}
            </Pressable>
          ))}
        </ScrollView>
      </View>
    </Modal>
  );
}
