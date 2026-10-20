// The dictation recording on disk (expo-audio writes it to the app's cache directory). Read once for
// transcription, then deleted: audio is never kept on the phone after it is transcribed or discarded.
import { File } from "expo-file-system";

export async function readRecording(uri: string): Promise<Uint8Array> {
  return new Uint8Array(await new File(uri).arrayBuffer());
}

export function deleteRecording(uri: string | null | undefined): void {
  if (!uri) return;
  try {
    const f = new File(uri);
    if (f.exists) f.delete();
  } catch {
    // already gone
  }
}
