import type { ReactNode } from "react";
import { View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { Orbs } from "./Orbs";
import { Txt } from "./ui";

/** Instrument Serif empty state over the dawn orbs. */
export function EmptyState({
  title,
  body,
  children,
}: {
  title: string;
  body?: string;
  children?: ReactNode;
}) {
  const t = useTheme();
  return (
    <View
      style={{
        minHeight: 420,
        alignItems: "center",
        justifyContent: "center",
        padding: t.space[4],
        overflow: "hidden",
      }}
    >
      <Orbs intensity={0.8} />
      <Txt variant="empty" style={{ textAlign: "center" }} accessibilityRole="header">
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
