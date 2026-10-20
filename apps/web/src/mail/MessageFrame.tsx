// One sanitized HTML body in a sandboxed srcdoc frame (§14.3). The frame has an opaque origin (no
// `allow-same-origin`, no `allow-scripts`), so the app cannot read its height. Instead the same prepared HTML
// is laid out once more in an inert shadow root of the same width, with the same base CSS and no remote
// fetches beyond the images already allowed into the frame, and the frame takes that height (D-267). The
// measuring host can never paint over the app: it is transparent and paint-contained (D-399).
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { buildSrcdoc, frameCss, IFRAME_SANDBOX, MEASURE_HOST_STYLE, measurementHtml } from "./bodyHtml.ts";

const MIN_HEIGHT = 64;
const SLACK = 4;

export function MessageFrame({
  html,
  syncOrigin,
  title,
  mode = "light",
  paper = false,
}: {
  html: string;
  syncOrigin: string | null;
  title: string;
  /** Dark only for messages that set no colours of their own (`isPlainLooking`, D-399). */
  mode?: "light" | "dark";
  /** The frame shows as a light page on the dark theme: it keeps an inset (`frameCss`). */
  paper?: boolean;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [height, setHeight] = useState<number | null>(null);
  const srcdoc = useMemo(() => buildSrcdoc(html, syncOrigin, mode, paper), [html, syncOrigin, mode, paper]);

  useLayoutEffect(() => {
    const el = frame.current;
    if (!el || typeof document.createElement("div").attachShadow !== "function") return;
    const host = document.createElement("div");
    host.className = "ak-measure";
    for (const [k, v] of Object.entries(MEASURE_HOST_STYLE)) host.style.setProperty(k, v, "important");
    host.setAttribute("aria-hidden", "true");
    (host as HTMLElement & { inert: boolean }).inert = true;
    const root = host.attachShadow({ mode: "closed" });
    root.innerHTML = `<style>${frameCss(".m", "light", paper)}</style><div class="m">${measurementHtml(html)}</div>`;
    const body = root.querySelector(".m") as HTMLElement;
    document.body.appendChild(host);
    const measure = () => {
      host.style.setProperty(
        "width",
        `${el.clientWidth || el.parentElement?.clientWidth || 640}px`,
        "important",
      );
      const h = Math.ceil(body.getBoundingClientRect().height);
      if (h > 0) setHeight(Math.max(MIN_HEIGHT, h + SLACK));
    };
    measure();
    // Images change the height when they load; widths change it when the pane resizes.
    root.addEventListener("load", measure, true);
    root.addEventListener("error", measure, true);
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    ro?.observe(el);
    return () => {
      ro?.disconnect();
      host.remove();
    };
  }, [html, paper]);

  return (
    <iframe
      ref={frame}
      className="ak-message__frame"
      title={title}
      sandbox={IFRAME_SANDBOX}
      srcDoc={srcdoc}
      referrerPolicy="no-referrer"
      loading="eager"
      style={{ height: height ?? "var(--ak-frame-initial)" }}
      // Keys pressed inside the frame never reach the app. Clicking into a message (to select or copy) keeps
      // focus there while the pointer is over it; leaving hands the keyboard back to the list.
      onMouseLeave={() => {
        if (document.activeElement === frame.current)
          document.querySelector<HTMLElement>(".ak-threadlist")?.focus({ preventScroll: true });
      }}
    />
  );
}
