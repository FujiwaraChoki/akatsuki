// §5.5: the model picker. ↑/↓ move, Enter or click selects; the provenance tag cross-fades and
// the summary restreams. On first view it steps from Akatsuki managed down to llama3.3.
import { page } from "./state";
import { $, $$, after, reducedMotion, watch } from "./util";

export function initPicker(fig: HTMLElement) {
  const list = $("[data-mp-list]", fig)!;
  const rows = $$<HTMLElement>("[data-mp]", fig);
  const tag = $("[data-mp-tag]", fig)!;
  const sum = $("[data-mp-sum]", fig)!;
  const keys = $("[data-mp-keys]", fig)!;
  const by = $("[data-mp-by]", fig);
  const lines = $$("p", sum).map((p) => p.textContent ?? "");
  const motion = !reducedMotion();
  let hi = rows.findIndex((r) => r.classList.contains("is-sel"));
  let sel = hi;
  let stopStream: (() => void) | null = null;
  let demoed = false;

  const mark = () => {
    rows.forEach((r, i) => {
      r.classList.toggle("is-sel", i === sel);
      r.setAttribute("aria-selected", String(i === sel));
      r.style.background = i === hi && i !== sel ? "var(--surface)" : "";
    });
    const cur = rows[hi];
    if (cur) list.setAttribute("aria-activedescendant", cur.id);
  };
  const setTag = (text: string, fade: boolean) => {
    if (!fade || !motion) {
      tag.textContent = text;
      return;
    }
    tag.classList.add("is-fading");
    after(200, () => {
      tag.textContent = text;
      tag.classList.remove("is-fading");
    });
  };
  const stream = () => {
    stopStream?.();
    if (!motion) {
      sum.innerHTML = lines.map((l) => `<p>${l}</p>`).join("");
      return;
    }
    const words = lines.map((l) => l.split(" "));
    const ps = lines.map(() => document.createElement("p"));
    sum.replaceChildren(...ps);
    sum.classList.add("is-streaming");
    let li = 0;
    let wi = 0;
    let stop = false;
    stopStream = () => (stop = true);
    const tick = () => {
      if (stop) return;
      const line = words[li];
      if (!line) {
        sum.classList.remove("is-streaming");
        return;
      }
      wi++;
      ps[li]!.textContent = line.slice(0, wi).join(" ");
      if (wi >= line.length) {
        li++;
        wi = 0;
      }
      after(18, tick);
    };
    after(120, tick);
  };
  const select = (i: number, fade = true) => {
    hi = i;
    sel = i;
    mark();
    setTag(rows[i]?.dataset.tag ?? "", fade);
    if (by) by.textContent = rows[i]?.dataset.short ?? "";
    stream();
  };
  const restKeys = keys.innerHTML;
  let flashTimer: (() => void) | null = null;
  const flash = (glyph: string) => {
    flashTimer?.();
    keys.innerHTML = `<kbd class="kbd">${glyph}</kbd>`;
    flashTimer = after(240, () => (keys.innerHTML = restKeys));
  };

  list.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      hi = (hi + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length;
      flash(e.key === "ArrowDown" ? "↓" : "↑");
      mark();
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      flash("↵");
      select(hi);
    }
  });
  rows.forEach((r, i) => {
    r.addEventListener("click", () => select(i));
  });

  if (!motion) return;
  // Start the first-view demo from "Akatsuki managed", but only once the visitor has scrolled and
  // the figure is still below the fold, so nothing visible jumps.
  const prime = () => {
    const r = fig.getBoundingClientRect();
    if (r.top < window.innerHeight) return;
    hi = 0;
    sel = 0;
    mark();
    tag.textContent = rows[0]?.dataset.tag ?? "";
    if (by) by.textContent = rows[0]?.dataset.short ?? "";
    primed = true;
  };
  let primed = false;
  page.onArm.push(prime);
  watch(fig, [0, 0.5], (en) => {
    if (demoed || !primed || en.intersectionRatio < 0.5) return;
    demoed = true;
    [1, 2, 3].forEach((n, k) => {
      after(300 + k * 220, () => {
        hi = n;
        flash("↓");
        mark();
      });
    });
    after(300 + 3 * 220 + 160, () => {
      flash("↵");
      select(3);
    });
  });
}
