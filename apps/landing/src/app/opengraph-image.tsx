import { ImageResponse } from "next/og";
import { OgArtwork, ogFonts } from "@/lib/og";

export const dynamic = "force-static";
export const alt = "Akatsuki: clear your inbox by first light. The open-source Superhuman alternative.";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default async function OpengraphImage() {
  return new ImageResponse(<OgArtwork />, { ...size, fonts: await ogFonts() });
}
