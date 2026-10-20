import type { MetadataRoute } from "next";
import { PAGES, SITE } from "@/lib/site";

export const dynamic = "force-static";

export default function sitemap(): MetadataRoute.Sitemap {
  return PAGES.map((p) => ({
    url: `${SITE.url}${p.href === "/" ? "" : p.href}`,
    changeFrequency: p.changeFrequency,
    priority: p.priority,
  }));
}
