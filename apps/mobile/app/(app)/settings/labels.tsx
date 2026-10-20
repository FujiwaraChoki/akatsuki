// Settings, Labels (§4.2 `labels.create` / `labels.rename` / `labels.remove`; D-417 follow-up): the labels the user
// made, per mailbox. Rename and delete are ops the worker applies at the provider (§5); the list updates at once
// (optimistic) and a failure arrives as an op-failure toast. System labels and folders are not edited here.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { useMutation } from "convex/react";
import { router } from "expo-router";
import { useState } from "react";
import { ScrollView, View } from "react-native";
import { ActionMenu } from "../../../src/components/ActionMenu";
import { Header, HeaderButton } from "../../../src/components/Header";
import { SheetModal } from "../../../src/components/SheetModal";
import { Button, Choice, Field, Row, Screen, Section, Txt } from "../../../src/components/ui";
import { confirmAction } from "../../../src/lib/confirm";
import { errorText } from "../../../src/lib/errors";
import { newClientId } from "../../../src/lib/ids";
import { useShortcuts } from "../../../src/state/keyboard";
import { type LabelView, useMail } from "../../../src/state/mail";
import { showToast } from "../../../src/state/toasts";
import { useTheme } from "../../../src/theme/ThemeProvider";
import {
  deleteLabelMessage,
  LABEL_NAME_MAX,
  labelNameProblem,
  labelsByAccount,
} from "../../../src/vm/labels";

type Editing = { mode: "new"; accountId: string } | { mode: "rename"; label: LabelView };

function NameSheet({
  editing,
  siblings,
  accounts,
  onClose,
}: {
  editing: Editing;
  siblings: LabelView[];
  accounts: { _id: string; email: string }[];
  onClose: () => void;
}) {
  const t = useTheme();
  const create = useMutation(api.labels.create);
  const rename = useMutation(api.labels.rename).withOptimisticUpdate((store, { labelId, name }) => {
    const cur = store.getQuery(api.labels.list, {});
    if (cur)
      store.setQuery(
        api.labels.list,
        {},
        cur.map((l) => (l._id === labelId ? { ...l, name } : l)),
      );
  });
  const [name, setName] = useState(editing.mode === "rename" ? editing.label.name : "");
  const [accountId, setAccountId] = useState(
    editing.mode === "new" ? editing.accountId : (editing.label.accountId as string),
  );
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pool = siblings.filter((l) => l.accountId === accountId);

  const save = async () => {
    const problem = labelNameProblem(
      name,
      pool.map((l) => ({ _id: l._id as string, name: l.name })),
      editing.mode === "rename" ? (editing.label._id as string) : undefined,
    );
    if (problem) return setError(problem);
    const n = name.trim();
    setBusy(true);
    try {
      if (editing.mode === "new") {
        await create({ clientOpId: newClientId(), accountId: accountId as Id<"mailAccounts">, name: n });
        showToast({ id: "label", text: `Label “${n}” created` });
      } else if (n !== editing.label.name) {
        await rename({ clientOpId: newClientId(), labelId: editing.label._id, name: n });
        showToast({ id: "label", text: `Renamed to “${n}”` });
      }
      onClose();
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SheetModal visible presentationStyle="pageSheet" onRequestClose={onClose}>
      <ScrollView
        style={{ backgroundColor: t.colors.background }}
        contentContainerStyle={{ padding: t.space[2], gap: t.space[2] }}
        keyboardShouldPersistTaps="handled"
      >
        <Txt variant="title">{editing.mode === "new" ? "New label" : `Rename “${editing.label.name}”`}</Txt>
        <Field
          label="Name"
          value={name}
          onChangeText={(v) => {
            setName(v);
            setError(null);
          }}
          maxLength={LABEL_NAME_MAX}
          autoFocus
          onSubmitEditing={() => void save()}
          returnKeyType="done"
          error={error}
        />
        {editing.mode === "new" && accounts.length > 1 ? (
          <Section title="Mailbox">
            {accounts.map((a) => (
              <Choice
                key={a._id}
                title={a.email}
                selected={a._id === accountId}
                onPress={() => setAccountId(a._id)}
              />
            ))}
          </Section>
        ) : null}
        <Txt variant="meta" tone="muted">
          Changes go to your provider too. Outlook calls labels categories; IMAP servers call them keywords.
        </Txt>
        <Button
          title={editing.mode === "new" ? "Create label" : "Save"}
          variant="primary"
          busy={busy}
          onPress={() => void save()}
        />
        <Button title="Cancel" variant="ghost" onPress={onClose} />
      </ScrollView>
    </SheetModal>
  );
}

export default function LabelsSettings() {
  const t = useTheme();
  const mail = useMail();
  const [editing, setEditing] = useState<Editing | null>(null);
  const [menu, setMenu] = useState<{ label: LabelView; folder: boolean } | null>(null);
  const remove = useMutation(api.labels.remove).withOptimisticUpdate((store, { labelId }) => {
    const cur = store.getQuery(api.labels.list, {});
    if (cur)
      store.setQuery(
        api.labels.list,
        {},
        cur.filter((l) => l._id !== labelId),
      );
  });
  useShortcuts("list", { "nav.back": () => router.back() });
  const groups = labelsByAccount(
    mail.labels.map((l) => ({ ...l, _id: l._id as string, accountId: l.accountId as string })),
    mail.accounts.map((a) => ({ ...a, _id: a._id as string })),
  );
  const accounts = groups.map((g) => g.account);

  const confirmDelete = (label: LabelView) => {
    const m = deleteLabelMessage(label.name);
    confirmAction({
      title: m.title,
      body: m.body,
      confirm: "Delete",
      cancel: "Keep it",
      destructive: true,
      onConfirm: () =>
        remove({ clientOpId: newClientId(), labelId: label._id })
          .then(() => showToast({ id: "label", text: `Deleted “${label.name}”` }))
          .catch((err) => showToast({ id: "label", text: errorText(err), tone: "error" })),
    });
  };

  return (
    <Screen>
      <Header
        title="Labels"
        onBack={() => router.back()}
        right={
          accounts.length > 0 ? (
            <HeaderButton
              label="New"
              accent
              onPress={() => setEditing({ mode: "new", accountId: accounts[0]!._id })}
            />
          ) : null
        }
      />
      <ScrollView contentContainerStyle={{ paddingBottom: t.space[8] }}>
        {groups.length === 0 ? (
          <View style={{ padding: t.space[2] }}>
            <Txt variant="body" tone="muted">
              Connect a mailbox to use labels.
            </Txt>
          </View>
        ) : null}
        {groups.map((g) => (
          <Section
            key={g.account._id}
            title={groups.length > 1 ? g.account.email : "Your labels"}
            footer={
              g.labels.length === 0 && g.folders.length === 0
                ? "No labels yet. Tap New, or type a new name in the label picker."
                : null
            }
          >
            {g.labels.map((l) => {
              const view = mail.labels.find((x) => x._id === l._id)!;
              return (
                <Row
                  key={l._id}
                  title={l.name}
                  onPress={() => setMenu({ label: view, folder: false })}
                  accessibilityHint="Rename or delete"
                  right={
                    <HeaderButton label="Edit" onPress={() => setMenu({ label: view, folder: false })} />
                  }
                />
              );
            })}
          </Section>
        ))}
        {groups
          .filter((g) => g.folders.length > 0)
          .map((g) => (
            <Section
              key={`f:${g.account._id}`}
              title={groups.length > 1 ? `Folders in ${g.account.email}` : "Your folders"}
              footer="Folders can be renamed here. Delete them at your provider: deleting a folder there also deletes the mail in it."
            >
              {g.folders.map((l) => {
                const view = mail.labels.find((x) => x._id === l._id)!;
                return (
                  <Row
                    key={l._id}
                    title={l.name}
                    onPress={() => setMenu({ label: view, folder: true })}
                    accessibilityHint="Rename"
                    right={
                      <HeaderButton label="Edit" onPress={() => setMenu({ label: view, folder: true })} />
                    }
                  />
                );
              })}
            </Section>
          ))}
      </ScrollView>
      <ActionMenu
        visible={menu !== null}
        onClose={() => setMenu(null)}
        title={menu?.label.name}
        items={
          menu
            ? [
                // iOS cannot present a sheet or an alert while the menu is still closing.
                {
                  key: "rename",
                  label: "Rename",
                  onPress: () => setTimeout(() => setEditing({ mode: "rename", label: menu.label }), 350),
                },
                ...(menu.folder
                  ? []
                  : [
                      {
                        key: "delete",
                        label: "Delete",
                        destructive: true,
                        onPress: () => setTimeout(() => confirmDelete(menu.label), 350),
                      },
                    ]),
              ]
            : []
        }
      />
      {editing ? (
        <NameSheet
          editing={editing}
          siblings={mail.labels}
          accounts={accounts}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </Screen>
  );
}
