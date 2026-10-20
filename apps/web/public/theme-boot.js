// Applies the stored theme before the first paint. Mirrors src/lib/theme.ts (key "ak:theme").
(() => {
  try {
    const t = localStorage.getItem("ak:theme");
    if (t === "light" || t === "dark") document.documentElement.setAttribute("data-theme", t);
    const d = localStorage.getItem("ak:density");
    if (d === "compact" || d === "comfortable") document.documentElement.setAttribute("data-density", d);
  } catch (_) {
    /* storage unavailable: follow the system */
  }
})();
