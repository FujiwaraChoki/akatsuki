// Snippet picker in the composer (§4.2 `snippets.*`, D-295): search, most used first; tapping one inserts it at
// the cursor (or over the selection) with its variables filled in. "Manage snippets" opens Settings.

import { rankSnippets, snippetHtmlToText } from "@akatsuki/mail-model/snippets";
import { router } from "expo-router";
import { useMemo, useState } from "react";
import { FlatList, Pressable, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "../theme/ThemeProvider";
import { SheetModal } from "./SheetModal";
import { Button, Txt } from "./ui";

export interface SnippetRow {
  _id: string;
  name: string;
  shortcut?: string;
  subject?: string;
  bodyHtml: string;
  useCount?: number;
}

export function SnippetSheet({
  snippets,
  onPick,
  onClose,
}: {
  snippets: readonly SnippetRow[] | undefined;
  onPick: (s: SnippetRow) => void;
  onClose: () => void;
}) {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const [q, setQ] = useState("");
  const rows = useMemo(() => rankSnippets(snippets ?? [], q), [snippets, q]);
  return (
    <SheetModal visible transparent onRequestClose={onClose}>
      <Pressable
        accessibilityLabel="Close"
        style={{ flex: 1, backgroundColor: t.colors.scrim }}
        onPress={onClose}
      />
      <View
        accessibilityViewIsModal
        style={{
          maxHeight: "75%",
          backgroundColor: t.colors.background,
          borderTopLeftRadius: t.radius.xl,
          borderTopRightRadius: t.radius.xl,
          paddingTop: t.space[2],
          paddingBottom: insets.bottom + t.space[1],
        }}
      >
        <View style={{ paddingHorizontal: t.space[2], gap: t.space[1] }}>
          <Txt variant="title">Snippets</Txt>
          <TextInput
            accessibilityLabel="Search snippets"
            value={q}
            onChangeText={setQ}
            placeholder="Search or type a ;shortcut"
            placeholderTextColor={t.colors.muted}
            selectionColor={t.colors.dawn}
            autoCapitalize="none"
            autoCorrect={false}
            onSubmitEditing={() => rows[0] && onPick(rows[0])}
            style={[
              t.text.reading,
              {
                color: t.colors.text,
                minHeight: 44,
                paddingHorizontal: t.space["1_5"],
                borderRadius: t.radius.md,
                backgroundColor: t.colors.surface,
              },
            ]}
          />
        </View>
        <FlatList
          data={rows}
          keyExtractor={(s) => s._id}
          keyboardShouldPersistTaps="handled"
          ListEmptyComponent={
            <Txt variant="body" tone="muted" style={{ padding: t.space[2] }}>
              {snippets === undefined
                ? "Loading…"
                : (snippets?.length ?? 0) === 0
                  ? "No snippets yet. Make one in Settings, Snippets."
                  : "No snippet matches."}
            </Txt>
          }
          renderItem={({ item }) => (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Insert ${item.name}`}
              onPress={() => onPick(item)}
              style={({ pressed }) => ({
                paddingHorizontal: t.space[2],
                paddingVertical: t.space["1_5"],
                gap: 2,
                backgroundColor: pressed ? t.colors.selected : "transparent",
              })}
            >
              <View style={{ flexDirection: "row", gap: t.space[1], alignItems: "baseline" }}>
                <Txt variant="bodyStrong" style={{ flex: 1 }} numberOfLines={1}>
                  {item.name}
                </Txt>
                {item.shortcut ? (
                  <Txt variant="code" tone="muted">
                    ;{item.shortcut}
                  </Txt>
                ) : null}
              </View>
              <Txt variant="meta" tone="muted" numberOfLines={2}>
                {snippetHtmlToText(item.bodyHtml)}
              </Txt>
            </Pressable>
          )}
        />
        <View style={{ flexDirection: "row", paddingHorizontal: t.space[2], gap: t.space[1] }}>
          <Button
            title="Manage snippets"
            variant="ghost"
            compact
            onPress={() => {
              onClose();
              router.push("/settings/snippets");
            }}
          />
          <View style={{ flex: 1 }} />
          <Button title="Close" variant="ghost" compact onPress={onClose} />
        </View>
      </View>
    </SheetModal>
  );
}
