// Theme and density: a local choice that has not reached the server yet survives a reload and is re-sent,
// instead of being overwritten by the older server value (review round 2, D-400).
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import {
  PENDING_KEY,
  PENDING_MAX_AGE_MS,
  pendingAppearance,
  setAppearance,
  storedTheme,
  useSyncServerTheme,
} from "../src/lib/theme.ts";

const html = () => document.documentElement;

describe("theme pending saves", () => {
  it("applies at once and forgets the pending value when the save resolves", async () => {
    const save = vi.fn(() => Promise.resolve(null));
    const done = setAppearance({ theme: "dark" }, save);
    expect(html().getAttribute("data-theme")).toBe("dark");
    expect(pendingAppearance()).toEqual({ theme: "dark" });
    await done;
    expect(save).toHaveBeenCalledWith({ theme: "dark" });
    expect(localStorage.getItem(PENDING_KEY)).toBeNull();
  });

  it("keeps an unsaved choice over an older server value after a reload, and re-sends it", async () => {
    // The page reloads before the save lands: the promise never resolves.
    void setAppearance({ theme: "dark" }, () => new Promise(() => {}));
    // Next page load: the server still says "system".
    const save = vi.fn(() => Promise.resolve(null));
    const { rerender } = renderHook(({ t }) => useSyncServerTheme(t, "comfortable", save), {
      initialProps: { t: "system" as "system" | "dark" },
    });
    expect(html().getAttribute("data-theme")).toBe("dark");
    expect(storedTheme()).toBe("dark");
    await waitFor(() => expect(save).toHaveBeenCalledWith({ theme: "dark" }));
    expect(save).toHaveBeenCalledTimes(1);
    // The server echoes the value: nothing is pending any more.
    rerender({ t: "dark" });
    await waitFor(() => expect(localStorage.getItem(PENDING_KEY)).toBeNull());
  });

  it("follows a server change made elsewhere when nothing is pending", () => {
    html().setAttribute("data-theme", "dark");
    localStorage.setItem("ak:theme", "dark");
    renderHook(() => useSyncServerTheme("light", undefined));
    expect(html().getAttribute("data-theme")).toBe("light");
  });

  it("drops a pending value after a day", () => {
    localStorage.setItem(
      PENDING_KEY,
      JSON.stringify({ theme: "dark", at: Date.now() - PENDING_MAX_AGE_MS - 1 }),
    );
    expect(pendingAppearance()).toEqual({});
    renderHook(() => useSyncServerTheme("light", undefined, () => Promise.resolve()));
    expect(html().getAttribute("data-theme")).toBe("light");
  });

  it("keeps density pending separately from theme", async () => {
    void setAppearance({ density: "compact" }, () => new Promise(() => {}));
    await setAppearance({ theme: "light" }, () => Promise.resolve());
    expect(pendingAppearance()).toEqual({ density: "compact" });
  });
});
