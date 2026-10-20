import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import {
  Avatar,
  Badge,
  Button,
  Eyebrow,
  formatCount,
  Heading,
  IconButton,
  initialsOf,
  Kbd,
  List,
  ListRow,
  ListRowSnippet,
  ListRowTitle,
  Spinner,
  tintOf,
  UiConfigProvider,
  UnreadDot,
  usePrefersReducedMotion,
} from "../src/index.ts";
import { setReducedMotion } from "./setup.ts";

const linux = ({ children }: { children: React.ReactNode }) => (
  <UiConfigProvider platform="linux">{children}</UiConfigProvider>
);
const mac = ({ children }: { children: React.ReactNode }) => (
  <UiConfigProvider platform="mac">{children}</UiConfigProvider>
);

describe("Kbd", () => {
  it("formats shortcut ids per platform, from packages/shortcuts", () => {
    const { unmount } = render(<Kbd shortcut="palette.open" />, { wrapper: mac });
    expect(screen.getByText("⌘K", { selector: ".ak-kbd__label" })).toBeTruthy();
    unmount();
    render(<Kbd shortcut="palette.open" />, { wrapper: linux });
    expect(screen.getByText("Ctrl+K", { selector: ".ak-kbd__label" })).toBeTruthy();
  });

  it("renders sequences step by step and nothing for palette-only commands", () => {
    const { container } = render(
      <>
        <Kbd shortcut="go.inbox" />
        <Kbd shortcut="labels.remove" data-testid="none" />
      </>,
      { wrapper: linux },
    );
    const kbd = screen.getByText("G then I").closest("kbd")!;
    expect(kbd.querySelectorAll(".ak-kbd__key")).toHaveLength(2);
    expect(kbd.getAttribute("aria-label")).toBeNull();
    expect(kbd.querySelector(".ak-kbd__step")?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelector("[data-testid=none]")).toBeNull();
  });

  it("follows the Gmail keymap override", () => {
    render(
      <UiConfigProvider platform="linux" keymap="gmail">
        <Kbd keys="shift+e" />
      </UiConfigProvider>,
    );
    expect(screen.getByText("Shift+E", { selector: ".ak-kbd__label" })).toBeTruthy();
  });
});

describe("Button", () => {
  it("shows the shortcut and exposes it to assistive tech", () => {
    render(
      <Button variant="primary" shortcut="compose.send">
        Send
      </Button>,
      { wrapper: linux },
    );
    const b = screen.getByRole("button", { name: /Send/ });
    expect(b.className).toContain("ak-button--primary");
    expect(b.getAttribute("aria-keyshortcuts")).toBe("Control+Enter");
    expect(b.getAttribute("type")).toBe("button");
  });

  it("is busy and disabled while loading", async () => {
    let clicks = 0;
    render(
      <Button loading onClick={() => clicks++}>
        Save
      </Button>,
    );
    const b = screen.getByRole("button", { name: /Save/ });
    expect(b.getAttribute("aria-busy")).toBe("true");
    expect((b as HTMLButtonElement).disabled).toBe(true);
    await userEvent.click(b);
    expect(clicks).toBe(0);
    expect(screen.getByRole("status", { name: "Working" })).toBeTruthy();
  });

  it("omits aria-keyshortcuts for sequences", () => {
    render(<Button shortcut="go.inbox">Inbox</Button>, { wrapper: linux });
    expect(screen.getByRole("button").hasAttribute("aria-keyshortcuts")).toBe(false);
  });
});

describe("IconButton", () => {
  it("is labelled and shows a tooltip with its key on focus", async () => {
    render(<IconButton label="Archive" icon={<svg />} shortcut="thread.archive" />, { wrapper: linux });
    const b = screen.getByRole("button", { name: "Archive" });
    expect(b.getAttribute("aria-keyshortcuts")).toBe("E");
    await act(async () => {
      b.focus();
    });
    const tip = await screen.findByRole("tooltip");
    expect(tip.textContent).toContain("Archive");
    expect(tip.textContent).toContain("E");
  });

  it("reports its pressed state", () => {
    render(<IconButton label="Star" icon="*" pressed tooltip={false} />);
    expect(screen.getByRole("button", { name: "Star" }).getAttribute("aria-pressed")).toBe("true");
  });
});

describe("Badge, Avatar, Spinner, typography", () => {
  it("caps counts like mail.unreadCounts", () => {
    expect(formatCount(150)).toBe("99+");
    expect(formatCount(7)).toBe("7");
    const { container } = render(
      <>
        <Badge count={150} tone="accent" />
        <Badge count={0} />
        <Badge>New</Badge>
      </>,
    );
    expect(container.textContent).toBe("99+New");
    expect(container.querySelector(".ak-badge--label")?.textContent).toBe("New");
  });

  it("derives initials and a stable tint", () => {
    expect(initialsOf("Ada Lovelace", "ada@example.com")).toBe("AL");
    expect(initialsOf(undefined, "sam.example@example.org")).toBe("SE");
    expect(initialsOf("Émile", "e@x.org")).toBe("É");
    expect(initialsOf("東京 太郎", "t@x.org")).toBe("東太");
    expect(tintOf("ada@example.com")).toBe(tintOf("ADA@example.com"));
    render(<Avatar name="Ada Lovelace" email="ada@example.com" />);
    const a = screen.getByRole("img", { name: "Ada Lovelace" });
    expect(a.textContent).toBe("AL");
  });

  it("falls back to initials when the image fails", async () => {
    const { container } = render(<Avatar email="sam@example.org" src="https://img.invalid/a.png" />);
    const img = container.querySelector("img")!;
    expect(img.getAttribute("alt")).toBe("");
    await act(async () => {
      img.dispatchEvent(new Event("error"));
    });
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe("S");
  });

  it("announces the spinner and renders headings and labels", () => {
    render(
      <>
        <Spinner label="Loading inbox" />
        <Heading level={1} variant="subject">
          Offsite plans
        </Heading>
        <Eyebrow>Splits</Eyebrow>
      </>,
    );
    expect(screen.getByRole("status", { name: "Loading inbox" })).toBeTruthy();
    expect(screen.getByRole("heading", { level: 1, name: "Offsite plans" }).className).toContain(
      "ak-heading--subject",
    );
    expect(screen.getByText("Splits").className).toContain("ak-eyebrow--app");
  });
});

describe("List rows", () => {
  it("is a listbox of options with active, selected and unread states", () => {
    render(
      <List aria-label="Inbox" activeId="row-2" density="compact">
        <ListRow id="row-1" selected>
          <ListRowTitle>Ada</ListRowTitle>
          <ListRowSnippet>Offsite</ListRowSnippet>
        </ListRow>
        <ListRow id="row-2" active unread pending>
          <UnreadDot />
          <ListRowTitle>Sam</ListRowTitle>
        </ListRow>
      </List>,
    );
    const list = screen.getByRole("listbox", { name: "Inbox" });
    expect(list.getAttribute("aria-activedescendant")).toBe("row-2");
    expect(list.getAttribute("aria-multiselectable")).toBe("true");
    const [one, two] = screen.getAllByRole("option");
    expect(one!.getAttribute("aria-selected")).toBe("true");
    expect(two!.getAttribute("aria-current")).toBe("true");
    expect(two!.className).toContain("ak-row--unread");
    expect(two!.getAttribute("data-pending")).toBe("true");
    expect(screen.getByRole("img", { name: "Unread" })).toBeTruthy();
  });
});

describe("usePrefersReducedMotion", () => {
  it("follows the reduced-motion preference live (the orbs are gone, First Light D-480)", async () => {
    const { result } = renderHook(() => usePrefersReducedMotion());
    expect(result.current).toBe(false);
    await act(async () => setReducedMotion(true));
    expect(result.current).toBe(true);
  });
});
