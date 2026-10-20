// Dictation sheet in the composer (§12.1 `dictation`, §12.7, D-128). Mounted only while open, so the recorder and
// the microphone are untouched until the user opens it. Before anything is recorded it says where the audio will
// go (the egress label for `dictation`); the microphone permission is asked for on the first Record. Audio is
// transcribed with the user's dictation route and the file is deleted afterwards.

import {
  type RecordingOptions,
  RecordingPresets,
  requestRecordingPermissionsAsync,
  setAudioModeAsync,
  useAudioRecorder,
} from "expo-audio";
import { router } from "expo-router";
import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { Pressable, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { type AiEnv, type AiStatus, aiStatusText } from "../lib/ai";
import { transcribeRecording } from "../lib/dictation";
import { errorText } from "../lib/errors";
import * as haptics from "../lib/haptics";
import { deleteRecording } from "../lib/recordingFile";
import { useTheme } from "../theme/ThemeProvider";
import { dictationReducer, elapsedLabel, MAX_RECORDING_MS } from "../vm/dictation";
import { EgressLine } from "./Egress";
import { SheetModal } from "./SheetModal";
import { Button, Spinner, Txt } from "./ui";

/** Speech needs little: mono AAC at 64 kbit/s keeps ten minutes under 5 MB. */
const OPTIONS: RecordingOptions = {
  ...RecordingPresets.HIGH_QUALITY,
  numberOfChannels: 1,
  bitRate: 64_000,
  web: { mimeType: "audio/webm", bitsPerSecond: 64_000 },
};

export function DictationSheet({
  ai,
  onText,
  onClose,
}: {
  ai: AiEnv;
  onText: (text: string) => void;
  onClose: () => void;
}) {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const recorder = useAudioRecorder(OPTIONS);
  const [phase, dispatch] = useReducer(dictationReducer, { kind: "ready" });
  const [status, setStatus] = useState<AiStatus>("loading");
  const [now, setNow] = useState(() => Date.now());
  const abort = useRef<AbortController | null>(null);
  const stopTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const egress = ai.egress("dictation");

  useEffect(() => {
    let live = true;
    ai.check("dictation")
      .then((s) => live && setStatus(s))
      .catch((e: unknown) => {
        if (live) dispatch({ type: "fail", message: errorText(e) });
      });
    return () => {
      live = false;
    };
  }, [ai]);

  useEffect(() => {
    if (phase.kind !== "recording") return;
    const id = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(id);
  }, [phase.kind]);

  const release = useCallback(async () => {
    if (stopTimer.current) clearTimeout(stopTimer.current);
    stopTimer.current = null;
    await setAudioModeAsync({ allowsRecording: false }).catch(() => {});
  }, []);

  const stop = useCallback(async () => {
    dispatch({ type: "stop" });
    let uri: string | null = null;
    try {
      await recorder.stop();
      uri = recorder.uri;
    } catch (e) {
      dispatch({ type: "fail", message: errorText(e) });
      await release();
      return;
    }
    await release();
    if (!uri) {
      dispatch({ type: "fail", message: "Nothing was recorded." });
      return;
    }
    const controller = new AbortController();
    abort.current = controller;
    try {
      const text = (await transcribeRecording(ai, uri, controller.signal)).trim();
      dispatch({ type: "text" });
      haptics.success();
      if (text) onText(text);
      onClose();
    } catch (e) {
      if (controller.signal.aborted) return;
      dispatch({ type: "fail", message: e instanceof Error && e.message ? e.message : errorText(e) });
    } finally {
      deleteRecording(uri);
    }
  }, [recorder, release, ai, onText, onClose]);

  const start = useCallback(async () => {
    dispatch({ type: "start" });
    try {
      const perm = await requestRecordingPermissionsAsync();
      if (!perm.granted) {
        dispatch({
          type: "fail",
          message: "Microphone access is off for Akatsuki. Turn it on in the system settings to dictate.",
        });
        return;
      }
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });
      await recorder.prepareToRecordAsync();
      recorder.record();
      haptics.tick();
      dispatch({ type: "recording", at: Date.now() });
      // §8: ten minutes at most; stop and transcribe what there is.
      stopTimer.current = setTimeout(() => void stop(), MAX_RECORDING_MS);
    } catch (e) {
      await release();
      dispatch({ type: "fail", message: errorText(e) });
    }
  }, [recorder, release, stop]);

  const cancel = useCallback(async () => {
    abort.current?.abort();
    if (phase.kind === "recording" || phase.kind === "starting") {
      await recorder.stop().catch(() => {});
      deleteRecording(recorder.uri);
    }
    await release();
    dispatch({ type: "close" });
    onClose();
  }, [phase.kind, recorder, release, onClose]);

  useEffect(
    () => () => {
      abort.current?.abort();
      if (stopTimer.current) clearTimeout(stopTimer.current);
    },
    [],
  );

  const blocked = status !== "ready" && status !== "loading";
  const recording = phase.kind === "recording";

  return (
    <SheetModal visible transparent onRequestClose={() => void cancel()}>
      <Pressable
        accessibilityLabel="Close dictation"
        style={{ flex: 1, backgroundColor: t.colors.scrim }}
        onPress={() => {
          if (!recording && phase.kind !== "transcribing") void cancel();
        }}
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
        <Txt variant="title">Dictate</Txt>
        {egress ? <EgressLine text={egress} full /> : null}
        {blocked ? (
          <>
            <Txt variant="body" tone="secondary">
              {aiStatusText(status as Exclude<AiStatus, "ready">)}
            </Txt>
            <Button
              title="Open AI settings"
              onPress={() => {
                onClose();
                router.push("/settings/ai");
              }}
            />
          </>
        ) : (
          <>
            <Txt variant="meta" tone="muted">
              {recording
                ? "Speak now. Tap Stop when you are done; the text goes in where your cursor was."
                : phase.kind === "transcribing"
                  ? "Turning your recording into text…"
                  : "Nothing is recorded until you tap Record. The recording is deleted from this phone once it is transcribed."}
            </Txt>
            {recording ? (
              <View style={{ flexDirection: "row", alignItems: "center", gap: t.space[1] }}>
                {/* The recording dot is one of the few red marks (spec §2.4). */}
                <View style={{ width: 8, height: 8, borderRadius: 4, backgroundColor: t.colors.dawn }} />
                <Txt variant="code" style={{ fontSize: 17 }} accessibilityLiveRegion="polite">
                  {`Listening… ${elapsedLabel(now - phase.startedAt)}`}
                </Txt>
              </View>
            ) : null}
            {phase.kind === "transcribing" || phase.kind === "starting" ? <Spinner /> : null}
            {phase.kind === "error" ? (
              <Txt variant="meta" tone="error" accessibilityLiveRegion="assertive">
                {phase.message}
              </Txt>
            ) : null}
            {recording ? (
              <Button title="Stop" variant="primary" onPress={() => void stop()} />
            ) : (
              <Button
                title={phase.kind === "error" ? "Record again" : "Record"}
                variant="primary"
                disabled={status !== "ready" || phase.kind === "transcribing" || phase.kind === "starting"}
                onPress={() => void start()}
              />
            )}
          </>
        )}
        <Button title="Cancel" variant="ghost" onPress={() => void cancel()} />
      </View>
    </SheetModal>
  );
}
