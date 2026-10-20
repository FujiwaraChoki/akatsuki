import "./doc.css";
import type { Metadata } from "next";
import { ModText } from "@/components/Kbd";
import { Shell } from "@/components/Shell";

export const metadata: Metadata = { title: "Not found", robots: { index: false } };

export default function NotFound() {
  return (
    <Shell current="">
      <div className="container nf">
        <p className="ph-crumb">
          <a href="/">Akatsuki</a>
          <span aria-hidden="true"> / </span>
          <span>404</span>
        </p>
        <h1 className="display nf-h1">Not in this inbox</h1>
        <p className="nf-copy kbd-only">
          Press G then H to go home, or <ModText k="K" /> to look around.
        </p>
        <p className="nf-copy touch-only" style={{ ["--touch-display" as string]: "block" }}>
          The page you followed is not here. Go home, or use the menu to look around.
        </p>
        <a className="tlink" href="/">
          Go home <span className="arr">→</span>
        </a>
      </div>
    </Shell>
  );
}
