// Attachments on the phone (§10.4): files from the system document picker or photos and videos from the library,
// uploaded raw to the worker's spool (`POST /v1/uploads`). The bytes are read by `expo-file-system`'s `File`,
// which is a `Blob` that `expo/fetch` streams as the request body, so a large video is never held as a JS string.
import * as DocumentPicker from "expo-document-picker";
import { File } from "expo-file-system";
import * as ImagePicker from "expo-image-picker";
import { type ComposeAttachment, MAX_ATTACHMENT_BYTES } from "../vm/compose";
import { requireWorker } from "./clients";
import { newClientId } from "./ids";

/** A picked file before it is uploaded. */
export interface PickedFile {
  uri: string;
  filename: string;
  size: number;
  mimeType: string;
}

/** The system document picker; any type, several at once. Copies to the cache so the file stays readable. */
export async function pickDocuments(): Promise<PickedFile[]> {
  const r = await DocumentPicker.getDocumentAsync({
    multiple: true,
    copyToCacheDirectory: true,
    type: "*/*",
  });
  if (r.canceled) return [];
  return r.assets.map((a) => ({
    uri: a.uri,
    filename: a.name || "attachment",
    size: a.size ?? 0,
    mimeType: a.mimeType || "application/octet-stream",
  }));
}

/** The photo library: photos and videos, several at once, sent as they are (no re-encoding). */
export async function pickPhotos(): Promise<PickedFile[]> {
  const r = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ["images", "videos"],
    allowsMultipleSelection: true,
    quality: 1,
  });
  if (r.canceled) return [];
  return r.assets.map((a, i) => ({
    uri: a.uri,
    filename: a.fileName || `photo-${i + 1}.${a.type === "video" ? "mp4" : "jpg"}`,
    size: a.fileSize ?? 0,
    mimeType: a.mimeType || (a.type === "video" ? "video/mp4" : "image/jpeg"),
  }));
}

/** The attachment row for a picked file, `uploading` (or `error` when it is over the limit). */
export function pendingAttachment(f: PickedFile): ComposeAttachment {
  const base = { key: newClientId(), filename: f.filename, size: f.size, mimeType: f.mimeType };
  return f.size > MAX_ATTACHMENT_BYTES
    ? { ...base, status: "error", error: "Over 25 MB." }
    : { ...base, status: "uploading" };
}

/** Uploads a picked file; resolves to the upload id. */
export async function uploadPicked(f: PickedFile): Promise<string> {
  const file = new File(f.uri);
  const r = await requireWorker().upload(file, f.filename, { contentType: f.mimeType });
  return r.uploadId;
}
