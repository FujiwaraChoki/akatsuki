import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AkatsukiUiProvider,
  Button,
  Dialog,
  Menu,
  MenuContent,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  MenuTrigger,
  type ToastApi,
  type ToastOptions,
  UNDO_TOAST_MS,
  useToast,
} from "../src/index.ts";

afterEach(() => vi.useRealTimers());

function ToastHarness({ onApi }: { onApi: (api: ToastApi) => void }) {
  const api = useToast();
  useEffect(() => {
    onApi(api);
  }, [api, onApi]);
  return null;
}

function renderToasts(max?: number) {
  let api!: ToastApi;
  render(
    <AkatsukiUiProvider platform="linux" maxToasts={max}>
      <ToastHarness onApi={(a) => (api = a)} />
    </AkatsukiUiProvider>,
  );
  const show = (o: ToastOptions) => {
    let id = "";
    act(() => {
      id = api.toast(o);
    });
    return id;
  };
  return { show, api: () => api };
}

describe("Toast", () => {
  it("shows an undo toast with its key and auto-dismisses after the undo window", () => {
    vi.useFakeTimers();
    const undo = vi.fn();
    const { show } = renderToasts();
    show({
      title: "Archived 3 conversations.",
      action: { label: "Undo", onAction: undo, shortcut: "undo" },
      durationMs: UNDO_TOAST_MS,
    });
    const status = screen.getByRole("status");
    expect(status.textContent).toContain("Archived 3 conversations.");
    expect(screen.getByText("Z", { selector: ".ak-kbd__label" })).toBeTruthy();
    act(() => vi.advanceTimersByTime(UNDO_TOAST_MS - 1));
    expect(screen.queryByRole("status")).not.toBeNull();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole("status")).toBeNull();
    expect(undo).not.toHaveBeenCalled();
  });

  it("dedupes by id, keeps errors until dismissed, and announces them assertively", async () => {
    vi.useFakeTimers();
    const { show } = renderToasts();
    show({ id: "op-1", title: "First", tone: "error" });
    show({
      id: "op-1",
      title: "Couldn't archive 'Offsite' in sami@…: the message no longer exists.",
      tone: "error",
    });
    const alerts = screen.getAllByRole("alert");
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.getAttribute("aria-live")).toBe("assertive");
    expect(alerts[0]!.textContent).toContain("Offsite");
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    vi.useRealTimers();
    await userEvent.click(screen.getByRole("button", { name: "Dismiss notification" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("runs an action, then dismisses; Retry and Dismiss can sit side by side", async () => {
    const retry = vi.fn();
    const dismissed = vi.fn();
    const { show } = renderToasts();
    show({
      id: "op-2",
      title: "Couldn't star.",
      tone: "error",
      action: { label: "Retry", onAction: retry },
      secondaryAction: { label: "Dismiss", onAction: () => {} },
      onDismiss: dismissed,
    });
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
    expect(dismissed).toHaveBeenCalledOnce();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps at most `max` toasts, dropping the oldest", () => {
    const gone = vi.fn();
    const { show, api } = renderToasts(2);
    show({ title: "one", onDismiss: gone });
    show({ title: "two" });
    show({ title: "three" });
    expect(screen.getAllByRole("status").map((s) => s.textContent?.slice(0, 5))).toEqual(["two×", "three"]);
    expect(gone).toHaveBeenCalledOnce();
    act(() => api().dismissAll());
    expect(screen.queryAllByRole("status")).toHaveLength(0);
  });

  it("pauses while the pointer is over the toasts", () => {
    vi.useFakeTimers();
    const { show } = renderToasts();
    show({ title: "Saved.", durationMs: 1000 });
    const region = screen.getByRole("region", { name: "Notifications" });
    act(() => vi.advanceTimersByTime(600));
    fireEvent.pointerEnter(region);
    act(() => vi.advanceTimersByTime(5000));
    expect(screen.queryByRole("status")).not.toBeNull();
    // Leaving resumes with the 400 ms that were left.
    fireEvent.pointerLeave(region);
    act(() => vi.advanceTimersByTime(399));
    expect(screen.queryByRole("status")).not.toBeNull();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("throws a clear error outside the provider", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => render(<ToastHarness onApi={() => {}} />)).toThrow(/ToastProvider/);
    spy.mockRestore();
  });
});

describe("Menu", () => {
  it("opens from the keyboard, shows shortcuts and selects", async () => {
    const onArchive = vi.fn();
    const user = userEvent.setup();
    render(
      <AkatsukiUiProvider platform="linux">
        <Menu>
          <MenuTrigger>
            <Button>More</Button>
          </MenuTrigger>
          <MenuContent>
            <MenuLabel>Conversation</MenuLabel>
            <MenuItem shortcut="thread.archive" onSelect={onArchive}>
              Archive
            </MenuItem>
            <MenuSeparator />
            <MenuItem destructive shortcut="thread.trash" onSelect={() => {}}>
              Move to Trash
            </MenuItem>
          </MenuContent>
        </Menu>
      </AkatsukiUiProvider>,
    );
    const trigger = screen.getByRole("button", { name: "More" });
    trigger.focus();
    await user.keyboard("{Enter}");
    const menu = await screen.findByRole("menu");
    expect(menu.textContent).toContain("Conversation");
    const archive = screen.getByRole("menuitem", { name: /Archive/ });
    expect(archive.textContent).toContain("E");
    expect(screen.getByRole("menuitem", { name: /Trash/ }).className).toContain("ak-menu__item--destructive");
    await user.keyboard("{Enter}");
    expect(onArchive).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

describe("Dialog", () => {
  function DialogHarness({ onChange }: { onChange: (open: boolean) => void }) {
    const [open, setOpen] = useState(true);
    return (
      <Dialog
        open={open}
        onOpenChange={(o) => {
          setOpen(o);
          onChange(o);
        }}
        title="Connect a mailbox"
        description="Akatsuki never stores message text on its servers."
        footer={<Button variant="primary">Continue</Button>}
      >
        <input aria-label="Email" />
      </Dialog>
    );
  }

  it("is a labelled, described modal that closes on Escape", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(<DialogHarness onChange={onChange} />);
    const dialog = await screen.findByRole("dialog", { name: "Connect a mailbox" });
    expect(dialog.getAttribute("aria-describedby")).toBeTruthy();
    expect(document.getElementById(dialog.getAttribute("aria-describedby")!)!.textContent).toContain(
      "never stores message text",
    );
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "Email" }));
    await user.keyboard("{Escape}");
    expect(onChange).toHaveBeenCalledWith(false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("can hide its title visually", async () => {
    render(<Dialog open title="Search" hideTitle onOpenChange={() => {}} />);
    expect(await screen.findByRole("dialog", { name: "Search" })).toBeTruthy();
  });
});
