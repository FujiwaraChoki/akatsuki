// Empty states (spec §4.8): a serif 32 title and a 15 muted line at about a third of the height, nothing else.
// Inbox zero is the one earned moment: serif 36 over the static horizon.
import type { ReactNode } from "react";
import { View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { Horizon } from "./Brand";
import { Txt } from "./ui";

export function EmptyState({
  title,
  body,
  children,
  zero,
}: {
  title: string;
  body?: string;
  children?: ReactNode;
  /** Inbox zero: the horizon and the larger title. */
  zero?: boolean;
}) {
  const t = useTheme();
  return (
    <View
      style={{
        minHeight: 460,
        alignItems: "center",
        paddingTop: 120,
        paddingHorizontal: t.space[4],
        overflow: "hidden",
      }}
    >
      {zero ? <Horizon style={{ top: 300 }} /> : null}
      <Txt
        variant="empty"
        style={[
          { textAlign: "center" },
          zero ? { fontSize: 36, lineHeight: 36, letterSpacing: -0.72 } : null,
        ]}
        accessibilityRole="header"
      >
        {title}
      </Txt>
      {body ? (
        <Txt
          variant="body"
          tone="muted"
          style={{ textAlign: "center", marginTop: t.space[1], maxWidth: 320 }}
        >
          {body}
        </Txt>
      ) : null}
      {children ? (
        <View style={{ marginTop: t.space[3], gap: t.space[1], alignItems: "stretch" }}>{children}</View>
      ) : null}
    </View>
  );
}
