// The app against the mock backend: sign-in, onboarding, keyboard navigation, optimistic actions with undo,
// the command palette, search, op-failure toasts, the sandboxed body frame, theme and Inbox Zero.
import { SYS } from "@akatsuki/mail-model";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "../src/app/App.tsx";
import { navigate } from "../src/app/router.tsx";
import { createMockBackend, DEMO_OTP } from "../src/backend/mock/index.tsx";
import { memoryStore } from "../src/lib/kv.ts";
import { BodyCache } from "../src/mail/bodyCache.ts";

function setup(opts: { signedIn?: boolean; onboarded?: boolean; path?: string } = {}) {
  const backend = createMockBackend({
    signedIn: opts.signedIn ?? true,
    onboarded: opts.onboarded ?? true,
    persist: false,
    latencyMs: 20,
  });
  if (opts.path) window.history.replaceState(null, "", opts.path);
  const user = userEvent.setup();
  const utils = render(
    <App
      backend={backend}
      syncOrigin="http://localhost:3000"
      bodyCache={new BodyCache({ store: memoryStore() })}
    />,
  );
  return { backend, user, ...utils };
}

const list = () => screen.getByRole("listbox", { name: /conversations/i });
const activeRow = () => {
  const id = list().getAttribute("aria-activedescendant");
  return id ? document.getElementById(id) : null;
};
const key = (k: string, init: KeyboardEventInit = {}) =>
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  });

describe("sign-in", () => {
  it("signs in with an email code", async () => {
    const { user } = setup({ signedIn: false, onboarded: false });
    await waitFor(() => expect(window.location.pathname).toBe("/signin"));
    expect(screen.getByRole("heading", { name: "Mail at the speed of thought." })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Continue with Google" })).toBeTruthy();
    await user.type(screen.getByLabelText("Email"), "you@example.com");
    await user.click(screen.getByRole("button", { name: "Email me a code" }));
    const code = await screen.findByLabelText("Code");
    // Keyboard-first: the code field has focus as soon as it appears.
    await waitFor(() => expect(document.activeElement).toBe(code));
    await user.keyboard("000000");
    expect((code as HTMLInputElement).value).toBe("000000");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect((await screen.findByRole("alert")).textContent).toContain(
      "That code is not right, or it has expired.",
    );
    await user.clear(code);
    await user.type(code, DEMO_OTP);
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    // New accounts land on onboarding (connect a mailbox).
    expect(await screen.findByRole("heading", { name: "Bring your inbox." })).toBeTruthy();
    expect(window.location.pathname).toBe("/onboarding");
  });

  it("rejects an invalid address before calling the server", async () => {
    const { user } = setup({ signedIn: false });
    await user.click(await screen.findByRole("button", { name: "Email me a code" }));
    expect(screen.getByRole("alert").textContent).toContain("Enter a valid email address.");
  });
});

describe("onboarding", () => {
  it("connects IMAP with autodetected servers", async () => {
    const { user } = setup({ onboarded: false, path: "/onboarding/imap" });
    await user.type(await screen.findByLabelText("Email address"), "me@fastmail.com");
    await user.click(screen.getByLabelText("Password"));
    expect(
      await screen.findByText(/Settings found for this provider\. This provider needs an app password/),
    ).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Server settings" }));
    expect((screen.getByLabelText("Incoming (IMAP) host") as HTMLInputElement).value).toBe(
      "imap.fastmail.com",
    );
    expect((screen.getByLabelText("Outgoing (SMTP) port") as HTMLInputElement).value).toBe("465");
    await user.type(screen.getByLabelText("Password"), "app-password");
    await user.click(screen.getByRole("button", { name: "Connect" }));
    await waitFor(() => expect(window.location.pathname).toBe("/inbox"));
  });

  it("redeems the OAuth grant the worker left in the URL (D-395)", async () => {
    const backend = createMockBackend({ signedIn: true, onboarded: false, persist: false, latencyMs: 20 });
    const url = new URL(
      await backend.worker.connectStart("google", { returnTo: "http://localhost:3000/onboarding" }),
    );
    expect(url.searchParams.get("status")).toBe("pending");
    window.history.replaceState(null, "", `/onboarding${url.search}`);
    render(
      <App
        backend={backend}
        syncOrigin="http://localhost:3000"
        bodyCache={new BodyCache({ store: memoryStore() })}
      />,
    );
    await waitFor(() => expect(window.location.pathname).toBe("/inbox"));
    expect(window.location.search).toBe("");
  });

  it("an unknown or foreign grant adds nothing and says so", async () => {
    setup({ onboarded: false, path: "/onboarding?status=pending&grant=someone-elses" });
    expect((await screen.findByRole("alert")).textContent).toContain("This connection expired.");
    expect(window.location.search).toBe("");
  });

  it("shows the IMAP error copy", async () => {
    const { user } = setup({ onboarded: false, path: "/onboarding/imap" });
    await user.type(await screen.findByLabelText("Email address"), "me@fastmail.com");
    await user.type(screen.getByLabelText("Password"), "wrong");
    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect((await screen.findByRole("alert")).textContent).toMatch(/password|sign in|log in/i);
  });
});

describe("mail", () => {
  it("moves with j/k, opens with Enter and closes with Escape", async () => {
    setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    const first = activeRow();
    expect(first).not.toBeNull();
    key("j");
    const second = activeRow();
    expect(second).not.toBe(first);
    key("k");
    expect(activeRow()).toBe(first);
    key("Enter");
    const id = first!.getAttribute("data-thread-id")!;
    await waitFor(() => expect(window.location.pathname).toBe(`/inbox/t/${id}`));
    const article = await screen.findByRole("article");
    expect(within(article).getByRole("heading", { level: 1 })).toBeTruthy();
    key("Escape");
    await waitFor(() => expect(window.location.pathname).toBe("/inbox"));
  });

  it("the list has the keyboard; Tab switches splits only from the list (D-399)", async () => {
    setup({ path: "/inbox" });
    const box = await screen.findByRole("listbox", { name: /Inbox conversations/ });
    await waitFor(() => expect(document.activeElement).toBe(box));
    expect(screen.getByRole("main")).toBeTruthy();
    // On a sidebar control Tab is the browser's: not prevented, no split change.
    const button = screen.getByRole("navigation", { name: "Mailboxes" }).querySelector("button")!;
    button.focus();
    const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    act(() => {
      button.dispatchEvent(tab);
    });
    expect(tab.defaultPrevented).toBe(false);
    expect(window.location.pathname).toBe("/inbox");
    // On the list it is "Next split".
    box.focus();
    act(() => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    });
    await waitFor(() => expect(window.location.pathname).toBe("/inbox/split/splitvip"));
  });

  it("archives optimistically and undoes with z", async () => {
    const { backend } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("j"); // skip the demo thread that fails to archive
    const row = activeRow()!;
    const id = row.getAttribute("data-thread-id")!;
    key("e");
    await waitFor(() => expect(document.querySelector(`[data-thread-id="${id}"]`)).toBeNull());
    expect(backend.store.threads.get(id)!.row.labelKeys).not.toContain(SYS.inbox);
    expect(await screen.findByText("Archived.")).toBeTruthy();
    key("z");
    await waitFor(() => expect(document.querySelector(`[data-thread-id="${id}"]`)).not.toBeNull());
    expect(backend.store.threads.get(id)!.row.labelKeys).toContain(SYS.inbox);
  });

  it("stars, marks unread and selects with the keyboard", async () => {
    const { backend } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    const id = activeRow()!.getAttribute("data-thread-id")!;
    const starred = backend.store.threads.get(id)!.row.labelKeys.includes(SYS.starred);
    key("s");
    await waitFor(() =>
      expect(backend.store.threads.get(id)!.row.labelKeys.includes(SYS.starred)).toBe(!starred),
    );
    const unread = backend.store.threads.get(id)!.row.labelKeys.includes(SYS.unread);
    key("u");
    await waitFor(() =>
      expect(backend.store.threads.get(id)!.row.labelKeys.includes(SYS.unread)).toBe(!unread),
    );
    key("x");
    key("j");
    key("x");
    expect(await screen.findByText("2 selected")).toBeTruthy();
    key("Escape");
    await waitFor(() => expect(screen.queryByText("2 selected")).toBeNull());
  });

  it("shows an op failure once with Retry and Dismiss, and the thread comes back", async () => {
    const { backend } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    const failing = [...backend.store.threads.values()].find((t) => t.failsOnArchive)!;
    expect(activeRow()!.getAttribute("data-thread-id")).toBe(failing.row._id);
    key("e");
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/^Couldn't archive 'Shared folder moved/);
    expect(alert.textContent).toContain("in you@…");
    expect(within(alert).getByRole("button", { name: "Retry" })).toBeTruthy();
    expect(within(alert).getByRole("button", { name: "Dismiss" })).toBeTruthy();
    await waitFor(() =>
      expect(document.querySelector(`[data-thread-id="${failing.row._id}"]`)).not.toBeNull(),
    );
  });

  it("opens the command palette with every action of the screen", async () => {
    const { user } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("k", { ctrlKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Command palette" });
    for (const label of ["Archive", "Snooze", "Move to Trash", "Go to Starred", "Use dark theme", "Sign out"])
      expect(within(dialog).getByRole("option", { name: new RegExp(`^${label}`) })).toBeTruthy();
    await user.type(within(dialog).getByRole("combobox"), "dark theme");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(document.documentElement.getAttribute("data-theme")).toBe("dark"));
  });

  it("goes to views with g sequences and cycles split tabs", async () => {
    setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("g");
    key("s");
    await waitFor(() => expect(window.location.pathname).toBe("/starred"));
    key("g");
    key("i");
    await waitFor(() => expect(window.location.pathname).toBe("/inbox"));
    key("Tab");
    await waitFor(() => expect(window.location.pathname).toBe("/inbox/split/splitvip"));
    expect(screen.getByRole("tab", { name: /VIP/ }).getAttribute("aria-selected")).toBe("true");
  });

  it("searches with the grammar and suggestions", async () => {
    const { user } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("/");
    const input = screen.getByRole("combobox", { name: "Search mail" });
    await waitFor(() => expect(document.activeElement).toBe(input));
    await user.type(input, "is:star");
    expect(screen.getByRole("option", { name: /is:starred/ })).toBeTruthy();
    await user.keyboard("{ArrowDown}{Enter}{Enter}");
    await waitFor(() => expect(window.location.pathname).toBe("/search"));
    expect(new URLSearchParams(window.location.search).get("q")).toBe("is:starred");
    await screen.findByRole("listbox", { name: /Search conversations/ });
  });

  it("snoozes from the picker and undoes it", async () => {
    const { backend, user } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("j");
    const id = activeRow()!.getAttribute("data-thread-id")!;
    key("h");
    const dialog = await screen.findByRole("dialog", { name: "Snooze until" });
    await user.click(within(dialog).getByRole("button", { name: /Tomorrow/ }));
    await waitFor(() => expect(backend.store.threads.get(id)!.row.labelKeys).toContain(SYS.snoozed));
    expect(await screen.findByText("Snoozed.")).toBeTruthy();
    key("z");
    await waitFor(() => expect(backend.store.threads.get(id)!.row.labelKeys).toContain(SYS.inbox));
  });

  it("renders bodies in a sandboxed frame with images deferred", async () => {
    const { backend, user } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    const t = [...backend.store.threads.values()].find(
      (x) => x.row.labelKeys.includes(SYS.inbox) && !x.row.isBulk,
    )!;
    act(() => navigate(`/inbox/t/${t.row._id}`));
    const frame = (await screen.findByTitle(/^Message from/)) as HTMLIFrameElement;
    expect(frame.getAttribute("sandbox")).toBe("allow-popups allow-popups-to-escape-sandbox");
    const srcdoc = frame.getAttribute("srcdoc")!;
    expect(srcdoc).toMatch(
      /^<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'/,
    );
    expect(srcdoc).not.toMatch(/<img[^>]+ src=/);
    await user.click(screen.getByRole("button", { name: /Show images/ }));
    await waitFor(() =>
      expect(
        (screen.getAllByTitle(/^Message from/).at(-1) as HTMLIFrameElement).getAttribute("srcdoc"),
      ).toMatch(/<img[^>]+ src="data:image\/png/),
    );
  });

  it("shows Inbox Zero when the inbox is empty", async () => {
    const { backend } = setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    const ids = [...backend.store.threads.values()]
      .filter((t) => t.row.labelKeys.includes(SYS.inbox))
      .map((t) => t.row._id);
    act(() => {
      backend.store.apply("bulk-trash", ids as never, { kind: "trash" });
    });
    // First Light (SPEC §3.3): "Inbox zero", no period (only the sign-in headline ends on the dot).
    expect(await screen.findByRole("heading", { name: "Inbox zero" })).toBeTruthy();
  });

  it("lists every shortcut with ?", async () => {
    setup({ path: "/inbox" });
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    fireEvent.keyDown(window, { key: "?" });
    const dialog = await screen.findByRole("dialog", { name: "Keyboard shortcuts" });
    expect(within(dialog).getByText("Archive")).toBeTruthy();
    expect(within(dialog).getByText("Go to Inbox")).toBeTruthy();
    // F6 is listed, the dialog has a visible Close button, and the dialog (the one scroll container) has focus.
    expect(within(dialog).getByText("Move between panes")).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(dialog));
  });

  it("moves focus to the list or the empty state after changing mailboxes", async () => {
    setup({ path: "/inbox" });
    const list = await screen.findByRole("listbox", { name: /Inbox conversations/ });
    await waitFor(() => expect(document.activeElement).toBe(list));
    // g then d: Drafts is empty in the mock mailbox; focus lands on the empty state, not <body>.
    fireEvent.keyDown(window, { key: "g" });
    fireEvent.keyDown(window, { key: "d" });
    await waitFor(() => expect(document.activeElement?.className).toContain("ak-listpane__empty"));
    fireEvent.keyDown(window, { key: "g" });
    fireEvent.keyDown(window, { key: "i" });
    const back = await screen.findByRole("listbox", { name: /Inbox conversations/ });
    await waitFor(() => expect(document.activeElement).toBe(back));
  });
});
