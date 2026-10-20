// Command palette: every action the app implements, from the shared key map (packages/shortcuts), filtered for the
// screen underneath; plus pickers for "Go to label", "Label" and "Move to folder" (for one conversation, or for a
// multi-select when opened with `bulk=1`; the targets come from state/targets.ts).
import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { SYS } from "@akatsuki/mail-model";
import type { Scope } from "@akatsuki/shortcuts";
import { useMutation } from "convex/react";
import { router, useLocalSearchParams } from "expo-router";
import { useMemo, useRef, useState } from "react";
import { FlatList, Pressable, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { NO_OUTLINE, Txt } from "../../src/components/ui";
import { errorText } from "../../src/lib/errors";
import { newClientId } from "../../src/lib/ids";
import { signOutEverywhereOnDevice } from "../../src/lib/session";
import { useThreadActions } from "../../src/state/actions";
import { useKeyboard, useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { finishPickerTargets, pickerTargets } from "../../src/state/targets";
import { showToast } from "../../src/state/toasts";
import { useTheme } from "../../src/theme/ThemeProvider";
import { mobileCommands } from "../../src/vm/keyboard";
import {
  APP_COMMAND_PREFIX,
  appPaletteItems,
  CREATE_LABEL_ID,
  filterPalette,
  movePaletteCursor,
  type PaletteItem,
  withCreateLabel,
} from "../../src/vm/palette";

type Mode = "commands" | "labels" | "label" | "move";

export default function Palette() {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const mail = useMail();
  const keyboard = useKeyboard();
  const actions = useThreadActions();
  const params = useLocalSearchParams<{
    mode?: string;
    threadId?: string;
    accountId?: string;
    scopes?: string;
    bulk?: string;
  }>();
  // A multi-select's conversations (captured once; the list clears its selection when the picker acts).
  const bulk = useRef(params.bulk === "1" ? pickerTargets() : []).current;
  const bulkAccounts = useMemo(() => new Set(bulk.map((b) => b.accountId)), [bulk]);
  const singleAccount =
    bulk.length > 0 ? (bulkAccounts.size === 1 ? bulk[0]!.accountId : null) : params.accountId;
  const mode: Mode =
    params.mode === "labels" || params.mode === "label" || params.mode === "move" ? params.mode : "commands";
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  // Scopes of the screen that opened the palette (captured once, before this screen registers its own).
  const scopes = useRef<Set<Scope>>(
    new Set(
      ((params.scopes ? params.scopes.split(",") : keyboard.activeScopes()) as Scope[]).filter(
        (s) => s !== "palette" && s !== "global",
      ),
    ),
  );

  const items = useMemo<PaletteItem[]>(() => {
    if (mode === "commands") {
      return [
        ...mobileCommands(keyboard.platform, keyboard.keymap, scopes.current).map((c) => ({
          id: c.id,
          label: c.label,
          group: c.group,
          hint: c.hint,
        })),
        ...appPaletteItems({
          theme: mail.prefs?.theme ?? "system",
          density: mail.prefs?.density ?? "comfortable",
        }),
      ];
    }
    const accountLabels = mail.labels.filter((l) =>
      bulk.length > 0 ? bulkAccounts.has(l.accountId) : !params.accountId || l.accountId === params.accountId,
    );
    if (mode === "move") {
      return [
        { id: SYS.inbox, label: "Inbox", group: "Folders", hint: null },
        ...accountLabels
          .filter((l) => l.exclusive)
          .map((l) => ({ id: l.labelKey, label: l.name, group: "Folders", hint: null })),
      ];
    }
    const userLabels = accountLabels.filter(
      (l) => l.labelKey.startsWith("l:") && (mode === "labels" || !l.exclusive),
    );
    return userLabels.map((l) => ({
      id: l.labelKey,
      label: l.name,
      group: mail.accountById.get(l.accountId)?.email ?? "Labels",
      hint: null,
    }));
  }, [
    mode,
    keyboard.platform,
    keyboard.keymap,
    mail.labels,
    mail.accountById,
    params.accountId,
    bulk.length,
    bulkAccounts,
    mail.prefs?.theme,
    mail.prefs?.density,
  ]);
  const updatePrefs = useMutation(api.me.updatePrefs);

  /** Rows without a key binding: appearance, settings pages, sign out. */
  const runApp = (id: string) => {
    const patch = (p: Parameters<typeof updatePrefs>[0]["patch"]) =>
      updatePrefs({ patch: p }).catch((err) =>
        showToast({ id: "prefs", text: errorText(err), tone: "error" }),
      );
    const rest = id.slice(APP_COMMAND_PREFIX.length);
    if (rest.startsWith("theme.")) return void patch({ theme: rest.slice(6) as "system" | "light" | "dark" });
    if (rest.startsWith("density."))
      return void patch({ density: rest.slice(8) as "compact" | "comfortable" });
    if (rest === "settings") return router.push("/settings");
    if (rest === "settings.ai") return router.push("/settings/ai");
    if (rest === "settings.shortcuts") return router.push("/settings/shortcuts");
    if (rest === "settings.snippets") return router.push("/settings/snippets");
    if (rest === "mailboxes") return router.push("/mailboxes");
    if (rest === "signOut") void signOutEverywhereOnDevice().then(() => router.replace("/sign-in"));
  };

  const filtered = useMemo(
    () =>
      mode === "label" && (params.threadId || bulk.length > 0) && singleAccount
        ? withCreateLabel(filterPalette(items, query), items, query)
        : filterPalette(items, query),
    [items, query, mode, params.threadId, bulk.length, singleAccount],
  );
  const createLabel = useMutation(api.labels.create);

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    router.back();
    if (mode === "commands") {
      // Run after the palette closed, so the screen underneath has focus and its handlers again.
      setTimeout(
        () => (item.id.startsWith(APP_COMMAND_PREFIX) ? runApp(item.id) : keyboard.run(item.id)),
        250,
      );
      return;
    }
    if (mode === "labels") {
      router.navigate({ pathname: "/inbox", params: { labelKey: item.id, title: item.label } });
      return;
    }
    if (bulk.length > 0) {
      chooseBulk(item);
      return;
    }
    if (!params.threadId || !params.accountId) return;
    const thread = { _id: params.threadId, accountId: params.accountId };
    if (item.id === CREATE_LABEL_ID) {
      // labels.create, then the label goes on the conversation (the worker orders the two ops, §5.4).
      const name = query.trim();
      createLabel({ clientOpId: newClientId(), accountId: params.accountId as Id<"mailAccounts">, name })
        .then(({ labelId }) => actions.labelAction(thread, labelId, true))
        .catch((err) => showToast({ id: "label-create", text: errorText(err), tone: "error" }));
      return;
    }
    if (mode === "move") actions.moveTo(thread, item.id, item.label);
    else actions.labelAction(thread, item.id.slice(2), true);
  };

  /** Label or move a multi-select. A label or folder belongs to one mailbox; Inbox works for all of them. */
  const chooseBulk = (item: PaletteItem) => {
    const name = query.trim();
    if (item.id === CREATE_LABEL_ID) {
      if (!singleAccount) return;
      createLabel({ clientOpId: newClientId(), accountId: singleAccount as Id<"mailAccounts">, name })
        .then(({ labelId }) => actions.labelMany(bulk, labelId, singleAccount))
        .catch((err) => showToast({ id: "label-create", text: errorText(err), tone: "error" }));
    } else if (mode === "move" && item.id === SYS.inbox) actions.moveMany(bulk, SYS.inbox, "Inbox");
    else {
      const label = mail.labels.find((l) => l.labelKey === item.id);
      if (!label) return;
      if (mode === "move") actions.moveMany(bulk, item.id, item.label, label.accountId);
      else actions.labelMany(bulk, label._id, label.accountId);
    }
    finishPickerTargets();
  };

  useShortcuts("palette", {
    "palette.close": () => router.back(),
    "palette.next": () => setCursor((c) => movePaletteCursor(filtered.length, c, 1)),
    "palette.prev": () => setCursor((c) => movePaletteCursor(filtered.length, c, -1)),
    "palette.run": () => choose(filtered[cursor]),
  });

  const placeholder =
    mode === "commands"
      ? "Type a command"
      : mode === "labels"
        ? "Go to label"
        : mode === "move"
          ? bulk.length > 0
            ? `Move ${bulk.length} to folder`
            : "Move to folder"
          : bulk.length > 0
            ? `Label ${bulk.length} conversations`
            : "Add a label";

  return (
    <View
      style={{
        flex: 1,
        paddingTop: insets.top + t.space[4],
        paddingHorizontal: t.space[2],
        backgroundColor: t.colors.scrimLight,
      }}
    >
      <Pressable
        accessibilityLabel="Close command palette"
        onPress={() => router.back()}
        style={{ position: "absolute", top: 0, left: 0, right: 0, bottom: 0 }}
      />
      <View
        accessibilityViewIsModal
        style={{
          alignSelf: "center",
          width: "100%",
          maxWidth: 640,
          maxHeight: "75%",
          borderRadius: t.radius.lg,
          backgroundColor: t.colors.background,
          borderWidth: 1,
          borderColor: t.colors.border,
          overflow: "hidden",
          boxShadow: t.shadow.pop,
        }}
      >
        <TextInput
          accessibilityLabel={placeholder}
          value={query}
          onChangeText={(v) => {
            setQuery(v);
            setCursor(0);
          }}
          onSubmitEditing={() => choose(filtered[cursor])}
          placeholder={placeholder}
          placeholderTextColor={t.colors.muted}
          selectionColor={t.colors.dawn}
          cursorColor={t.colors.dawn}
          autoFocus
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="go"
          style={[
            t.text.reading,
            {
              color: t.colors.text,
              minHeight: 52,
              paddingHorizontal: t.space[2],
              borderBottomWidth: 1,
              borderBottomColor: t.colors.borderSubtle,
            },
            NO_OUTLINE,
          ]}
        />
        <FlatList
          data={filtered}
          keyExtractor={(i) => i.id}
          keyboardShouldPersistTaps="handled"
          ListEmptyComponent={
            <Txt variant="body" tone="muted" style={{ padding: t.space[2] }}>
              {mode === "commands" ? "No matching command" : "Nothing found"}
            </Txt>
          }
          renderItem={({ item, index }) => (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={item.hint ? `${item.label}, shortcut ${item.hint}` : item.label}
              onPress={() => choose(item)}
              style={({ pressed }) => ({
                flexDirection: "row",
                alignItems: "center",
                minHeight: 48,
                paddingHorizontal: t.space[2],
                gap: t.space[1],
                backgroundColor: pressed || index === cursor ? t.colors.selected : "transparent",
              })}
            >
              <Txt variant="body" style={{ flex: 1 }} numberOfLines={1}>
                {item.label}
              </Txt>
              <Txt variant="meta" tone="muted">
                {item.group}
              </Txt>
              {item.hint ? (
                <Txt variant="code" tone="muted" style={{ minWidth: 40, textAlign: "right" }}>
                  {item.hint}
                </Txt>
              ) : null}
            </Pressable>
          )}
        />
      </View>
    </View>
  );
}
