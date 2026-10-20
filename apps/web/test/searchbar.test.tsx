// The search field shows parse errors only after a pause, never for an operator still waiting for its value.
import { UiConfigProvider } from "@akatsuki/ui";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ERROR_PAUSE_MS, SearchBar } from "../src/mail/SearchBar.tsx";

function Harness() {
  const [v, setV] = useState("");
  return (
    <UiConfigProvider platform="linux">
      <SearchBar
        value={v}
        onChange={setV}
        onSubmit={() => {}}
        onClose={() => {}}
        onFocusChange={() => {}}
        inputRef={null}
        context={{ contacts: [], labels: [], splits: [] }}
        onContactQuery={() => {}}
        timezone="UTC"
      />
    </UiConfigProvider>
  );
}

describe("search errors", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("waits for a pause and ignores a trailing operator", () => {
    render(<Harness />);
    const input = screen.getByRole("combobox", { name: "Search mail" });
    fireEvent.focus(input);
    fireEvent.change(input, { target: { value: "from:" } });
    act(() => vi.advanceTimersByTime(ERROR_PAUSE_MS * 3));
    expect(document.querySelector(".ak-search__error")).toBeNull();
    expect(input.getAttribute("aria-invalid")).toBeNull();

    fireEvent.change(input, { target: { value: "from: hello" } });
    expect(document.querySelector(".ak-search__error")).toBeNull();
    act(() => vi.advanceTimersByTime(ERROR_PAUSE_MS + 10));
    expect(document.querySelector(".ak-search__error")?.textContent).toMatch(/from/);
    expect(input.getAttribute("aria-invalid")).toBe("true");
  });
});
