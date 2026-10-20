import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { describe, expect, it } from "vitest";
import {
  CommandPalette,
  fuzzyMatch,
  highlightRuns,
  type PaletteItem,
  paletteItemsFromShortcuts,
  rankPaletteItems,
  UiConfigProvider,
} from "../src/index.ts";

const ran: string[] = [];
const item = (
  id: string,
  label: string,
  section?: string,
  extra: Partial<PaletteItem> = {},
): PaletteItem => ({
  id,
  label,
  section,
  run: () => ran.push(id),
  ...extra,
});

const items: PaletteItem[] = [
  item("thread.archive", "Archive", "Conversation", { shortcut: "thread.archive" }),
  item("thread.trash", "Move to Trash", "Conversation", { shortcut: "thread.trash", keywords: ["delete"] }),
  item("go.inbox", "Go to Inbox", "Navigation", { shortcut: "go.inbox" }),
  item("go.sent", "Go to Sent", "Navigation", { shortcut: "go.sent" }),
  item("account.switch", "Switch account", undefined, { hint: "sami@…" }),
  item("ai.off", "Summarize conversation", "AI", { disabled: true }),
];

function Harness({ onClose, recent }: { onClose?: () => void; recent?: string[] }) {
  const [open, setOpen] = useState(true);
  return (
    <UiConfigProvider platform="linux">
      <CommandPalette
        open={open}
        onOpenChange={(o) => {
          setOpen(o);
          if (!o) onClose?.();
        }}
        items={items}
        recent={recent}
      />
    </UiConfigProvider>
  );
}

describe("fuzzyMatch", () => {
  it("prefers word-start substrings, then subsequences", () => {
    const archive = fuzzyMatch("arch", "Archive")!;
    const inbox = fuzzyMatch("arch", "Search archive")!;
    expect(archive.score).toBeGreaterThan(inbox.score);
    expect(fuzzyMatch("mtt", "Move to Trash")?.indices).toEqual([0, 5, 8]);
    expect(fuzzyMatch("xyz", "Archive")).toBeNull();
    expect(fuzzyMatch("", "Anything")).toEqual({ score: 0, indices: [] });
    // Every token must match, in any order; keywords count.
    expect(fuzzyMatch("trash move", "Move to Trash")).not.toBeNull();
    expect(fuzzyMatch("delete", "Move to Trash", ["delete"])).not.toBeNull();
  });

  it("splits labels into highlight runs", () => {
    expect(highlightRuns("Archive", [0, 1])).toEqual([
      { text: "Ar", match: true },
      { text: "chive", match: false },
    ]);
  });
});

describe("rankPaletteItems", () => {
  it("groups by section in input order, with recent first", () => {
    const sections = rankPaletteItems(items, "", ["go.sent"]);
    expect(sections.map((s) => s.name)).toEqual(["Recent", "Conversation", "Navigation", "Commands", "AI"]);
    expect(sections[2]!.rows.map((r) => r.item.id)).toEqual(["go.inbox"]);
  });

  it("orders sections by their best match", () => {
    const sections = rankPaletteItems(items, "inbox");
    expect(sections[0]!.rows[0]!.item.id).toBe("go.inbox");
  });
});

describe("paletteItemsFromShortcuts", () => {
  it("offers only commands that have a handler, with their keys", () => {
    const out = paletteItemsFromShortcuts(
      { "thread.archive": () => {}, "go.inbox": () => {}, "palette.open": () => {} },
      { platform: "linux" },
    );
    // palette.open is not a palette command (palette: false).
    expect(out.map((i) => [i.id, i.section, i.shortcut])).toEqual([
      ["go.inbox", "Navigation", "go.inbox"],
      ["thread.archive", "Conversation", "thread.archive"],
    ]);
  });
  it("carries the key map's synonyms, so 'new email' finds Compose and 'print' Open original", () => {
    const out = paletteItemsFromShortcuts(
      { "compose.new": () => {}, "thread.openOriginal": () => {}, "thread.trash": () => {} },
      { platform: "linux" },
    );
    const best = (q: string) => rankPaletteItems(out, q, [])[0]?.rows[0]?.item.id;
    expect(best("new email")).toBe("compose.new");
    expect(best("print")).toBe("thread.openOriginal");
    expect(best("delete")).toBe("thread.trash");
  });
});

describe("CommandPalette", () => {
  it("is a labelled combobox with sections and key hints", async () => {
    render(<Harness />);
    const input = await screen.findByRole("combobox", { name: "Command palette" });
    expect(document.activeElement).toBe(input);
    const list = screen.getByRole("listbox", { name: "Commands" });
    expect(within(list).getByRole("group", { name: "Conversation" })).toBeTruthy();
    const archive = within(list).getByRole("option", { name: /Archive/ });
    expect(archive.getAttribute("aria-selected")).toBe("true");
    expect(input.getAttribute("aria-activedescendant")).toBe(archive.id);
    expect(within(list).getByText("G then I", { selector: ".ak-kbd__label" })).toBeTruthy();
    expect(within(list).getByText("sami@…")).toBeTruthy();
  });

  it("filters, moves with the palette keys, skips disabled items and runs on Enter", async () => {
    ran.length = 0;
    let closed = 0;
    const user = userEvent.setup();
    render(<Harness onClose={() => closed++} />);
    const input = await screen.findByRole("combobox");
    await user.type(input, "go");
    const options = screen.getAllByRole("option");
    expect(options.map((o) => o.textContent?.startsWith("Go to"))).toContain(true);
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{Control>}p{/Control}");
    await user.keyboard("{Control>}n{/Control}");
    const active = screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true")!;
    await user.keyboard("{Enter}");
    expect(ran).toHaveLength(1);
    expect(active.textContent).toContain(items.find((i) => i.id === ran[0])!.label);
    expect(closed).toBe(1);
    expect(screen.queryByRole("combobox")).toBeNull();
  });

  it("shows an empty state, runs on click, and closes on Escape", async () => {
    ran.length = 0;
    const user = userEvent.setup();
    const { unmount } = render(<Harness />);
    const input = await screen.findByRole("combobox");
    await user.type(input, "zzzz");
    expect(screen.getByText("No matching commands.")).toBeTruthy();
    await user.clear(input);
    await user.click(screen.getByRole("option", { name: /Go to Sent/ }));
    expect(ran).toEqual(["go.sent"]);
    unmount();

    let closed = 0;
    render(<Harness onClose={() => closed++} />);
    await screen.findByRole("combobox");
    await user.keyboard("{Escape}");
    expect(closed).toBe(1);
  });

  it("never runs disabled items", async () => {
    ran.length = 0;
    const user = userEvent.setup();
    render(<Harness />);
    await screen.findByRole("combobox");
    const disabled = screen.getByRole("option", { name: /Summarize/ });
    expect(disabled.getAttribute("aria-disabled")).toBe("true");
    await act(async () => {
      disabled.click();
    });
    expect(ran).toEqual([]);
    await user.keyboard("{Control>}{End}{/Control}");
    const active = screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true")!;
    expect(active.textContent).toContain("Switch account");
  });
});

describe("CommandPalette focus", () => {
  function Field() {
    const [open, setOpen] = useState(false);
    useEffect(() => {
      // Ctrl+K from the field, as the mail shell does with a window listener.
      const onKey = (e: KeyboardEvent) => {
        if (e.ctrlKey && e.key === "k") {
          e.preventDefault();
          setOpen(true);
        }
      };
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, []);
    return (
      <UiConfigProvider platform="linux">
        <textarea aria-label="Message" />
        <CommandPalette open={open} onOpenChange={setOpen} items={[item("noop", "Do nothing")]} />
      </UiConfigProvider>
    );
  }

  it("gives focus back to the field it was opened from (Escape and after a command)", async () => {
    const user = userEvent.setup();
    render(<Field />);
    const field = screen.getByLabelText("Message");
    await user.click(field);
    await user.keyboard("Hi");
    await user.keyboard("{Control>}k{/Control}");
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("combobox")));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
    expect(document.activeElement).toBe(field);

    await user.keyboard("{Control>}k{/Control}");
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("combobox")));
    await user.keyboard("{Enter}");
    await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
    expect(document.activeElement).toBe(field);
    await user.keyboard("e");
    expect((field as HTMLTextAreaElement).value).toBe("Hie");
  });
});
