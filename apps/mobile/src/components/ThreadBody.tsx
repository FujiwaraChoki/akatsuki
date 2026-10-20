// The sandboxed thread view (ARCHITECTURE §14.3, mobile row): react-native-webview with JavaScript disabled, no
// storage, no cookies, no file access, no new windows, and the document CSP from vm/bodyDocument. Every navigation
// is intercepted: web and mail links open outside the app, attachment links go to the app, the rest is blocked.
import * as Linking from "expo-linking";
import { useRef } from "react";
import { WebView } from "react-native-webview";
import { useTheme } from "../theme/ThemeProvider";
import { linkPolicy } from "../vm/bodyDocument";

export interface ThreadBodyProps {
  html: string;
  onAttachment: (messageId: string, partId: string) => void;
}

export function ThreadBody({ html, onAttachment }: ThreadBodyProps) {
  const t = useTheme();
  const loaded = useRef(false);
  return (
    <WebView
      source={{ html }}
      style={{ flex: 1, backgroundColor: t.colors.background }}
      originWhitelist={["about:*", "data:*"]}
      javaScriptEnabled={false}
      javaScriptCanOpenWindowsAutomatically={false}
      domStorageEnabled={false}
      allowFileAccess={false}
      allowFileAccessFromFileURLs={false}
      allowUniversalAccessFromFileURLs={false}
      allowsLinkPreview={false}
      setSupportMultipleWindows={false}
      incognito
      cacheEnabled={false}
      thirdPartyCookiesEnabled={false}
      sharedCookiesEnabled={false}
      geolocationEnabled={false}
      mixedContentMode="never"
      dataDetectorTypes="none"
      mediaPlaybackRequiresUserAction
      allowsInlineMediaPlayback={false}
      allowsBackForwardNavigationGestures={false}
      textZoom={100}
      onLoadEnd={() => {
        loaded.current = true;
      }}
      onShouldStartLoadWithRequest={(req) => {
        const decision = linkPolicy(req.url, !loaded.current);
        switch (decision.kind) {
          case "load":
            return true;
          case "external":
            Linking.openURL(decision.url).catch(() => {});
            return false;
          case "attachment":
            onAttachment(decision.messageId, decision.partId);
            return false;
          case "block":
            return false;
        }
      }}
      accessibilityLabel="Conversation"
    />
  );
}
