// Write with AI and Rewrite in the composer (§12.1 `compose`, `rewrite`): a sheet with the instruction, quick
// prompts, the four rewrite styles and the "where your data goes" label of each feature. The composer runs the
// stream and shows the text arriving in the body; this sheet only collects what to do.

import { useState } from "react";
import { Pressable, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "../theme/ThemeProvider";
import {
  INSTRUCTION_MAX,
  instructionProblem,
  REWRITE_STYLES,
  type RewriteStyle,
  writeSuggestions,
} from "../vm/composeAi";
import { EgressLine } from "./Egress";
import { SheetModal } from "./SheetModal";
import { Button, Chip, Label, Txt } from "./ui";

export function ComposeAiSheet({
  isReply,
  canRewrite,
  rewriteScope,
  egress,
  onWrite,
  onRewrite,
  onClose,
}: {
  isReply: boolean;
  /** There is text to rewrite (the selection, else the body). */
  canRewrite: boolean;
  /** "the selected text" or "your message". */
  rewriteScope: string;
  egress: { compose: string | null; rewrite: string | null };
  onWrite: (instruction: string) => void;
  onRewrite: (style: RewriteStyle) => void;
  onClose: () => void;
}) {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const [instruction, setInstruction] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const submit = () => {
    const p = instructionProblem(instruction);
    setProblem(p);
    if (!p) onWrite(instruction.trim());
  };
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
          backgroundColor: t.colors.background,
          borderTopLeftRadius: t.radius.xl,
          borderTopRightRadius: t.radius.xl,
          padding: t.space[2],
          paddingBottom: insets.bottom + t.space[2],
          gap: t.space["1_5"],
        }}
      >
        <Txt variant="title">Write with AI</Txt>
        {egress.compose ? <EgressLine text={egress.compose} full /> : null}
        <TextInput
          accessibilityLabel="What should it write?"
          value={instruction}
          onChangeText={(v) => {
            setInstruction(v);
            setProblem(null);
          }}
          onSubmitEditing={submit}
          placeholder={isReply ? "Accept and suggest Tuesday at 3" : "Ask Mara for the Q3 numbers by Friday"}
          placeholderTextColor={t.colors.muted}
          selectionColor={t.colors.dawn}
          autoFocus
          multiline
          maxLength={INSTRUCTION_MAX + 100}
          submitBehavior="submit"
          returnKeyType="go"
          style={[
            t.text.reading,
            {
              color: t.colors.text,
              minHeight: 64,
              padding: t.space["1_5"],
              borderRadius: t.radius.md,
              borderWidth: 1,
              borderColor: problem ? t.colors.accentText : t.colors.border,
            },
          ]}
        />
        {problem ? (
          <Txt variant="meta" tone="error" accessibilityLiveRegion="polite">
            {problem}
          </Txt>
        ) : null}
        <View style={{ flexDirection: "row", flexWrap: "wrap", gap: t.space["0_5"] }}>
          {writeSuggestions(isReply).map((s) => (
            <Chip key={s} label={s} onPress={() => setInstruction(s)} />
          ))}
        </View>
        <Button title="Write" variant="primary" onPress={submit} />
        <View style={{ gap: t.space["0_5"], marginTop: t.space[1] }}>
          <Label>Rewrite {rewriteScope}</Label>
          {egress.rewrite && egress.rewrite !== egress.compose ? (
            <EgressLine text={egress.rewrite} full />
          ) : null}
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: t.space["0_5"] }}>
            {REWRITE_STYLES.map((r) => (
              <Chip
                key={r.style}
                label={r.label}
                onPress={canRewrite ? () => onRewrite(r.style) : undefined}
                selected={false}
              />
            ))}
          </View>
          {!canRewrite ? (
            <Txt variant="meta" tone="muted">
              Write something first, then rewrite it.
            </Txt>
          ) : null}
        </View>
        <Button title="Cancel" variant="ghost" onPress={onClose} />
      </View>
    </SheetModal>
  );
}
