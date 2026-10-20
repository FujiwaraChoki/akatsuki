// The speed figure: one archive on four clocks, played back slowly once. The key-press stop is lit
// at once (that is the point); the line then runs out to the provider. Armed only if the figure is
// below the fold when the visitor starts scrolling, so full-page captures, crawlers and
// reduced-motion visitors see the finished state the server rendered.
import { page } from "./state";
import { $, $$, after, reducedMotion, watch } from "./util";

/** When each stop lights, in ms after the key press. Slowed down so it can be read. */
const STEP_AT = [0, 450, 1350, 2500];

export function initTiming(fig: HTMLElement) {
  if (reducedMotion()) return;
  const stops = $$<HTMLElement>("[data-tl-stop]", fig);
  const replay = $<HTMLButtonElement>("[data-tl-replay]", fig);
  let armed = false;
  let played = false;
  let timers: (() => void)[] = [];

  // CSS draws each segment of the line once the stop after it is done.
  const set = (n: number) => {
    stops.forEach((s, i) => {
      s.classList.toggle("is-done", i <= n);
    });
  };

  const reset = () => {
    for (const c of timers) c();
    timers = [];
    fig.classList.add("is-pending");
    set(-1);
  };

  const play = () => {
    reset();
    STEP_AT.forEach((t, i) => {
      timers.push(after(300 + t, () => set(i)));
    });
    timers.push(after(300 + (STEP_AT.at(-1) ?? 0) + 400, () => fig.classList.remove("is-pending")));
  };

  page.onArm.push(() => {
    if (fig.getBoundingClientRect().top < window.innerHeight) return;
    armed = true;
    reset();
  });

  watch(fig, [0, 0.5], (en) => {
    if (!armed || played || en.intersectionRatio < 0.5) return;
    played = true;
    play();
  });

  replay?.addEventListener("click", () => {
    played = true;
    play();
  });
  if (replay) replay.hidden = false;
}
