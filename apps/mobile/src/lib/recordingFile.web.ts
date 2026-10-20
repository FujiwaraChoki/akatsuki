// Web preview: expo-audio records into a blob: URL in the browser.
export async function readRecording(uri: string): Promise<Uint8Array> {
  const res = await fetch(uri);
  return new Uint8Array(await res.arrayBuffer());
}

export function deleteRecording(uri: string | null | undefined): void {
  if (uri?.startsWith("blob:")) URL.revokeObjectURL(uri);
}
