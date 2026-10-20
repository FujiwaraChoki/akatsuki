// Settings, Snippets (§4.2 `snippets.*`, D-295): reusable text with an optional `;shortcut` and `{{variables}}`.
// In the composer a snippet goes in from the Snippets picker (Ctrl/⌘ ; on a keyboard) or by typing its ;shortcut.
// Bodies are edited as plain text and stored as simple HTML (packages/mail-model/snippets); Convex sanitizes.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import {
  BUILTIN_VARIABLES,
  rankSnippets,
  snippetHtmlToText,
  snippetProblems,
  snippetSaveArgs,
  variablesIn,
} from "@akatsuki/mail-model/snippets";
import { useMutation, useQuery } from "convex/react";
import { router } from "expo-router";
import { useState } from "react";
import { ScrollView, TextInput, View } from "react-native";
import { Header, HeaderButton } from "../../../src/components/Header";
import { SheetModal } from "../../../src/components/SheetModal";
import { Button, Field, Label, Row, Screen, Section, Txt } from "../../../src/components/ui";
import { confirmAction } from "../../../src/lib/confirm";
import { errorText } from "../../../src/lib/errors";
import { useShortcuts } from "../../../src/state/keyboard";
import { useMail } from "../../../src/state/mail";
import { showToast } from "../../../src/state/toasts";
import { useTheme } from "../../../src/theme/ThemeProvider";

type SnippetView = (typeof api.snippets.list)["_returnType"][number];

function SnippetEditor({ snippet, onClose }: { snippet: SnippetView | null; onClose: () => void }) {
  const t = useTheme();
  const save = useMutation(api.snippets.save);
  const remove = useMutation(api.snippets.remove);
  const [name, setName] = useState(snippet?.name ?? "");
  const [shortcut, setShortcut] = useState(snippet?.shortcut ?? "");
  const [subject, setSubject] = useState(snippet?.subject ?? "");
  const [text, setText] = useState(snippet ? snippetHtmlToText(snippet.bodyHtml) : "");
  const [problems, setProblems] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const vars = variablesIn(text);
  const builtin = new Set<string>(BUILTIN_VARIABLES);
  const custom = vars.filter((v) => !builtin.has(v));

  const submit = async () => {
    const p = snippetProblems({ name, shortcut, text });
    setProblems(p);
    if (p.length > 0) return;
    setBusy(true);
    try {
      const args = snippetSaveArgs({
        ...(snippet ? { snippetId: snippet._id as string } : {}),
        name,
        shortcut,
        subject,
        text,
      });
      await save({ ...args, snippetId: args.snippetId as Id<"snippets"> | undefined });
      showToast({ id: "snippet", text: `Snippet “${args.name}” saved` });
      onClose();
    } catch (err) {
      setProblems([errorText(err)]);
    } finally {
      setBusy(false);
    }
  };

  const del = () => {
    if (!snippet) return;
    confirmAction({
      title: `Delete “${snippet.name}”?`,
      body: "It is removed from every device. Messages you already sent are not changed.",
      confirm: "Delete",
      destructive: true,
      onConfirm: () =>
        remove({ snippetId: snippet._id })
          .then(() => {
            showToast({ id: "snippet", text: `Deleted “${snippet.name}”` });
            onClose();
          })
          .catch((err) => setProblems([errorText(err)])),
    });
  };

  return (
    <SheetModal visible presentationStyle="pageSheet" onRequestClose={onClose}>
      <ScrollView
        style={{ backgroundColor: t.colors.background }}
        contentContainerStyle={{ padding: t.space[2], gap: t.space[2], paddingBottom: t.space[6] }}
        keyboardShouldPersistTaps="handled"
      >
        <Txt variant="title">{snippet ? "Edit snippet" : "New snippet"}</Txt>
        <Field label="Name" value={name} onChangeText={setName} maxLength={100} autoFocus={!snippet} />
        <Field
          label="Shortcut (optional)"
          value={shortcut}
          onChangeText={setShortcut}
          placeholder=";thanks"
          autoCapitalize="none"
          autoCorrect={false}
        />
        <Txt variant="meta" tone="muted">
          Type ;{shortcut.replace(/^;/, "") || "shortcut"} in a message to insert it.
        </Txt>
        <Field label="Subject (optional)" value={subject} onChangeText={setSubject} maxLength={998} />
        <View style={{ gap: t.space["0_5"] }}>
          <Label>Text</Label>
          <TextInput
            accessibilityLabel="Snippet text"
            value={text}
            onChangeText={setText}
            multiline
            textAlignVertical="top"
            placeholder="Hi {{first_name}}, thanks for …"
            placeholderTextColor={t.colors.muted}
            selectionColor={t.colors.dawn}
            style={[
              t.text.reading,
              {
                color: t.colors.text,
                minHeight: 160,
                padding: t.space["1_5"],
                borderRadius: t.radius.md,
                borderWidth: 1,
                borderColor: t.colors.border,
              },
            ]}
          />
          <Txt variant="meta" tone="muted">
            {
              "{{first_name}}, {{name}}, {{email}}, {{day}} and {{date}} fill in from the first recipient and today."
            }
            {custom.length > 0
              ? ` You will be asked to fill in ${custom.map((v) => `{{${v}}}`).join(", ")}.`
              : ""}
          </Txt>
        </View>
        {problems.map((p) => (
          <Txt key={p} variant="meta" tone="error" accessibilityLiveRegion="polite">
            {p}
          </Txt>
        ))}
        <Button title="Save" variant="primary" busy={busy} onPress={() => void submit()} />
        {snippet ? <Button title="Delete snippet" variant="ghost" onPress={del} /> : null}
        <Button title="Cancel" variant="ghost" onPress={onClose} />
      </ScrollView>
    </SheetModal>
  );
}

export default function SnippetsSettings() {
  const t = useTheme();
  const mail = useMail();
  const snippets = useQuery(api.snippets.list, mail.entitled ? {} : "skip");
  const [editing, setEditing] = useState<SnippetView | "new" | null>(null);
  useShortcuts("list", { "nav.back": () => router.back() });
  const rows = rankSnippets(
    (snippets ?? []).map((s) => ({ ...s, _id: s._id as string })),
    "",
  );

  return (
    <Screen>
      <Header
        title="Snippets"
        onBack={() => router.back()}
        right={<HeaderButton label="New" accent onPress={() => setEditing("new")} />}
      />
      <ScrollView contentContainerStyle={{ paddingBottom: t.space[8] }}>
        <Section
          title="Your snippets"
          footer="Reusable text for the composer. Insert one from Snippets in the composer, or type its ;shortcut."
        >
          {snippets === undefined ? <Row title="Loading…" /> : null}
          {snippets && rows.length === 0 ? (
            <Row title="No snippets yet" detail="Tap New to make one." />
          ) : null}
          {rows.map((s) => {
            const view = snippets!.find((x) => x._id === s._id)!;
            return (
              <Row
                key={s._id}
                title={s.shortcut ? `${s.name}  ;${s.shortcut}` : s.name}
                detail={snippetHtmlToText(s.bodyHtml).slice(0, 120)}
                onPress={() => setEditing(view)}
                accessibilityHint="Edit the snippet"
              />
            );
          })}
        </Section>
      </ScrollView>
      {editing ? (
        <SnippetEditor snippet={editing === "new" ? null : editing} onClose={() => setEditing(null)} />
      ) : null}
    </Screen>
  );
}
