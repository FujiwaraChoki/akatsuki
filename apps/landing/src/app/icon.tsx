import { ImageResponse } from "next/og";
import { IconArtwork } from "@/lib/og";

export const dynamic = "force-static";
export const size = { width: 64, height: 64 };
export const contentType = "image/png";

export default function Icon() {
  return new ImageResponse(<IconArtwork size={64} />, size);
}
