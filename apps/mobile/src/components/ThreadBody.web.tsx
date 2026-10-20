// Web preview: the same document in a sandboxed iframe (as apps/web does, §14.3): no `allow-scripts`, no
// `allow-same-origin`; links open in a new tab through `allow-popups`.
import { createElement } from "react";
import { View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";

export interface ThreadBodyProps {
  html: string;
  onAttachment: (messageId: string, partId: string) => void;
}

export function ThreadBody({ html }: ThreadBodyProps) {
  const t = useTheme();
  return (
    <View style={{ flex: 1, backgroundColor: t.colors.background }}>
      {createElement("iframe", {
        title: "Conversation",
        sandbox: "allow-popups allow-popups-to-escape-sandbox",
        referrerPolicy: "no-referrer",
        srcDoc: html,
        style: { border: "none", width: "100%", height: "100%", background: t.colors.background },
      })}
    </View>
  );
}
