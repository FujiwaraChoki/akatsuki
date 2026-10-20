// Part 2 in the app, against the mock backend: compose from the keyboard, undo-send with z, reply with r,
// snippets with variables, reminders, settings (read-receipt warning, keymap, splits, AI consent) and an AI
// summary through the managed path, all with no network.
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../src/app/App.tsx";
import { createMockBackend } from "../src/backend/mock/index.tsx";
import { setDraftStore } from "../src/compose/Composer.tsx";
import { resetSendFlow } from "../src/compose/sendFlow.ts";
import { composer } from "../src/compose/store.ts";
import { registerFeatures } from "../src/features/index.ts";
import { memoryStore } from "../src/lib/kv.ts";
import { BodyCache } from "../src/mail/bodyCache.ts";

registerFeatures();

function setup(path = "/inbox", opts: { plan?: "trialing" | "none" | "selfhost"; onboarded?: boolean } = {}) {
  const backend = createMockBackend({
    signedIn: true,
    onboarded: opts.onboarded ?? true,
    persist: false,
    latencyMs: 10,
    ...(opts.plan && { plan: opts.plan }),
  });
  window.history.replaceState(null, "", path);
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

const key = (k: string, init: KeyboardEventInit = {}) =>
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  });
/** Keys pressed inside a focused field (the composer listens on window, like the shell). */
const keyIn = (el: Element, k: string, init: KeyboardEventInit = {}) =>
  act(() => {
    el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  });

beforeEach(() => {
  composer.reset();
  resetSendFlow();
  setDraftStore(memoryStore());
});

describe("compose", () => {
  it("c opens the composer; Ctrl+Enter sends with Undo; z brings the draft back", async () => {
    const { user, backend } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("c");
    const dialog = await screen.findByRole("dialog", { name: "New message" });
    const to = within(dialog).getByRole("combobox", { name: "To" });
    await waitFor(() => expect(document.activeElement).toBe(to));
    await user.type(to, "lena@example.org,");
    expect(within(dialog).getByRole("button", { name: "Remove lena@example.org" })).toBeTruthy();
    await user.type(within(dialog).getByLabelText("Subject"), "Offsite photos");
    await user.type(within(dialog).getByLabelText("Message"), "Here they are.");
    keyIn(within(dialog).getByLabelText("Message"), "Enter", { ctrlKey: true });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "New message" })).toBeNull());
    expect(await screen.findByText("Sending…")).toBeTruthy();
    await waitFor(() => expect(backend.store.extras.sends.some((s) => s.state === "scheduled")).toBe(true));
    key("z");
    expect(await screen.findByText("Sending cancelled.")).toBeTruthy();
    const back = await screen.findByRole("dialog", { name: "New message" });
    expect((within(back).getByLabelText("Subject") as HTMLInputElement).value).toBe("Offsite photos");
    expect((within(back).getByLabelText("Message") as HTMLTextAreaElement).value).toBe("Here they are.");
    expect(backend.store.extras.sends.find((s) => s.preview.subject === "Offsite photos")?.state).toBe(
      "cancelled",
    );
  });

  it("explains what is missing instead of sending", async () => {
    const { user } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("c");
    const dialog = await screen.findByRole("dialog", { name: "New message" });
    await user.click(within(dialog).getByRole("button", { name: /^Send(?! later)/ }));
    expect(within(dialog).getByRole("alert").textContent).toBe("Add at least one recipient.");
  });

  it("r replies with the sender, the Re: subject and the quote; snippets fill variables", async () => {
    const { user, backend } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("j");
    key("Enter");
    await screen.findByRole("article");
    key("r");
    const dialog = await screen.findByRole("dialog", { name: "Reply" });
    expect((within(dialog).getByLabelText("Subject") as HTMLInputElement).value).toMatch(/^Re: /);
    expect(within(dialog).getAllByRole("button", { name: /^Remove / }).length).toBeGreaterThan(0);
    await within(dialog).findByRole("button", { name: "Show quoted text" });
    // Snippet by shortcut: ;thx then Tab.
    const body = within(dialog).getByLabelText("Message") as HTMLTextAreaElement;
    await user.type(body, ";thx");
    fireEvent.keyDown(body, { key: "Tab" });
    await waitFor(() =>
      expect(body.value).toMatch(/^Thanks \w+, I will review this and get back to you by \w+day\.$/),
    );
    expect(backend.store.extras.snippets.find((s) => s.shortcut === "thx")?.useCount).toBe(4);
  });

  it("autosaves to the provider draft after 2 s", async () => {
    const { user, backend } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("c");
    const dialog = await screen.findByRole("dialog", { name: "New message" });
    await user.type(within(dialog).getByLabelText("Subject"), "Plan");
    await waitFor(() => expect(within(dialog).getByText("Draft saved")).toBeTruthy(), { timeout: 4000 });
    const saved = [...backend.store.extras.drafts.values()][0];
    expect(saved?.meta.subject).toBe("Plan");
    expect(saved?.meta.version).toBe(1);
  });
});

describe("compose with AI", () => {
  const withAi = (backend: ReturnType<typeof setup>["backend"]) =>
    act(() => {
      backend.store.extras.saveAi({
        profiles: [
          {
            profileId: "11111111-1111-4111-8111-111111111111",
            label: "Akatsuki AI",
            kind: "akatsuki",
            models: {},
            keyOnDevices: [],
          },
        ],
        routes: [{ feature: "default", profileId: "11111111-1111-4111-8111-111111111111", enabled: true }],
        consent: { grantedAt: 1, version: 1, autoTriage: false, syncVoiceProfile: false },
      });
    });

  it("Ctrl+J writes the body from an instruction, with its egress label and an undo", async () => {
    const { user, backend } = setup();
    withAi(backend);
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("c");
    const dialog = await screen.findByRole("dialog", { name: "New message" });
    keyIn(within(dialog).getByRole("combobox", { name: "To" }), "j", { ctrlKey: true });
    const instruction = await within(dialog).findByLabelText("Write with AI");
    await within(dialog).findByText("Sent to Akatsuki for processing, not stored");
    await user.type(instruction, "say yes{Enter}");
    const body = within(dialog).getByLabelText("Message") as HTMLTextAreaElement;
    await waitFor(() =>
      expect(body.value).toBe(
        "Thanks for the update. That works for me; let's go ahead as planned and check in on Friday.",
      ),
    );
    await user.click(within(dialog).getByRole("button", { name: "Undo AI change" }));
    expect(body.value).toBe("");
  });

  it("rewrites the selection shorter", async () => {
    const { user, backend } = setup();
    withAi(backend);
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("c");
    const dialog = await screen.findByRole("dialog", { name: "New message" });
    const body = within(dialog).getByLabelText("Message") as HTMLTextAreaElement;
    await user.type(body, "First sentence here. Second one is long.");
    // First Light (D-487): the rewrite options live in the Write-with-AI popover, not a strip.
    await user.click(within(dialog).getByRole("button", { name: "Rewrite and dictate" }));
    await user.click(within(dialog).getByRole("button", { name: "Shorter" }));
    await waitFor(() => expect(body.value).toBe("First sentence here."));
  });

  it("an untouched reply closed with Escape leaves no draft", async () => {
    const { backend } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("Enter");
    await screen.findByRole("article");
    key("r");
    const dialog = await screen.findByRole("dialog", { name: "Reply" });
    keyIn(within(dialog).getByLabelText("Message"), "Escape");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Reply" })).toBeNull());
    await new Promise((r) => setTimeout(r, 50));
    expect(backend.store.extras.drafts.size).toBe(0);
  });
});

describe("reminders", () => {
  it("Shift+H sets a natural-language reminder that only fires without a reply", async () => {
    const { user, backend } = setup();
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("Enter");
    await screen.findByRole("article");
    key("H", { shiftKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Remind me" });
    await user.type(within(dialog).getByLabelText("When"), "next week");
    await user.type(within(dialog).getByLabelText("Note (optional)"), "Chase the contract");
    await user.click(within(dialog).getByRole("button", { name: "Set reminder" }));
    expect(await screen.findByText(/^Reminder set for .*, if nobody replies\.$/)).toBeTruthy();
    const timer = backend.store.timers.find((t) => t.kind === "reminder");
    expect(timer?.condition).toBe("if_no_reply");
    expect(await screen.findByText(/^Reminder .*, if nobody replies: Chase the contract$/)).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Cancel reminder" }));
    await waitFor(() => expect(backend.store.timers.some((t) => t.kind === "reminder")).toBe(false));
  });
});

describe("settings", () => {
  it("read receipts need the warning acknowledged", async () => {
    const { user, backend } = setup("/settings/mail");
    const sw = await screen.findByRole("switch", { name: "Read receipts" });
    await user.click(sw);
    const warning = screen.getByRole("alertdialog", { name: "Before you turn on read receipts" });
    expect(warning.textContent).toContain("Apple Mail Privacy Protection");
    const on = within(warning).getByRole("button", { name: "Turn on read receipts" }) as HTMLButtonElement;
    expect(on.disabled).toBe(true);
    await user.click(within(warning).getByRole("checkbox"));
    await user.click(on);
    await waitFor(() => expect(backend.store.me()?.profile.prefs.readReceipts.enabled).toBe(true));
    expect(backend.store.me()?.profile.prefs.readReceipts.warningAcknowledgedAt).toBeGreaterThan(0);
  });

  it("switches to the Gmail keymap", async () => {
    const { user, backend } = setup("/settings/shortcuts");
    await user.click(await screen.findByRole("radio", { name: "Gmail" }));
    await waitFor(() => expect(backend.store.me()?.profile.prefs.keymap).toBe("gmail"));
  });

  it("creates a split with structured conditions and reorders it", async () => {
    const { user, backend } = setup("/settings/splits");
    await user.click(await screen.findByRole("button", { name: "New split" }));
    await user.type(screen.getByLabelText("Name"), "Shop");
    await user.selectOptions(screen.getByLabelText("Condition 1 field"), "from_domain");
    await user.type(screen.getByLabelText("Condition 1 value"), "Shop.Example.net");
    await user.click(screen.getByRole("button", { name: "Save split" }));
    await waitFor(() => expect(backend.store.splits.some((s) => s.name === "Shop")).toBe(true));
    expect(backend.store.splits.find((s) => s.name === "Shop")?.rule).toEqual({
      match: "all",
      conds: [{ f: "from_domain", eq: "shop.example.net" }],
    });
    await user.click(await screen.findByRole("button", { name: "Move Shop up" }));
    await waitFor(() =>
      expect([...backend.store.splits].sort((a, b) => a.order - b.order).map((s) => s.name)).toEqual([
        "VIP",
        "Shop",
        "News",
      ]),
    );
  });

  it("Escape leaves settings for the inbox", async () => {
    setup("/settings/appearance");
    await screen.findByRole("heading", { name: "Appearance" });
    key("Escape");
    await waitFor(() => expect(window.location.pathname).toBe("/inbox"));
  });

  it("new hosted users start the trial first", async () => {
    setup("/", { plan: "none", onboarded: false });
    expect(await screen.findByRole("heading", { name: "Start your trial." })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Start 14-day free trial" })).toBeTruthy();
  });
});

describe("AI", () => {
  it("nothing runs before consent; then Akatsuki AI summarizes with its egress label", async () => {
    const { user } = setup("/settings/ai");
    await user.click(await screen.findByRole("button", { name: "Turn on AI features" }));
    await user.click(await screen.findByRole("button", { name: "Use Akatsuki AI" }));
    await screen.findByText("Akatsuki AI added.");
    key("Escape");
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("Enter");
    await screen.findByRole("article");
    expect(
      (await screen.findAllByText("Sent to Akatsuki for processing, not stored")).length,
    ).toBeGreaterThan(0);
    key("S", { shiftKey: true });
    expect(
      await screen.findByText(
        "They want your review of the attached notes before Friday's meeting.",
        {},
        { timeout: 4000 },
      ),
    ).toBeTruthy();
  });

  it("without consent the thread offers the settings link instead", async () => {
    setup("/inbox");
    await screen.findByRole("listbox", { name: /Inbox conversations/ });
    key("Enter");
    await screen.findByRole("article");
    expect(await screen.findByText(/AI is off\. Turn it on in Settings, AI\./)).toBeTruthy();
    // Shift+S says so too (a toast with a way there), instead of doing nothing.
    key("S", { shiftKey: true });
    const toast = await screen.findByRole("button", { name: "Open AI settings" });
    expect(screen.getAllByText(/AI is off\. Turn it on in Settings, AI\./).length).toBe(2);
    await act(async () => toast.click());
    await waitFor(() => expect(window.location.pathname).toBe("/settings/ai"));
  });
});
