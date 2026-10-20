// Smoke tests of part 2 (compose, undo-send, send later, settings, AI) on the production bundle with the mock
// backend, under the production CSP. No network: the mock stands in for Convex, the worker and apps/api.
import { expect, type Page, test } from "@playwright/test";

const CODE = "123456";

async function onboard(page: Page) {
  await page.goto("/");
  await page.getByLabel("Email").fill("you@example.com");
  await page.getByRole("button", { name: "Email me a code" }).click();
  await page.getByLabel("Code").fill(CODE);
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /Gmail/ }).click();
  await expect(page.getByRole("listbox", { name: "Inbox conversations" })).toBeVisible();
}

const composer = (page: Page) => page.getByRole("dialog", { name: /New message|Reply|Forward/ });

async function writeNew(page: Page, subject: string) {
  await page.keyboard.press("c");
  const c = composer(page);
  await expect(c).toBeVisible();
  await expect(c.getByLabel("To", { exact: true })).toBeFocused();
  await page.keyboard.type("mara@example.com");
  await page.keyboard.press("Enter");
  await c.getByLabel("Subject").fill(subject);
  await c.getByLabel("Message").fill("Hi Mara,\n\nThe notes are attached.\n\nBest");
  return c;
}

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  (page as Page & { errors?: string[] }).errors = errors;
});

test.afterEach(async ({ page }) => {
  const errors = ((page as Page & { errors?: string[] }).errors ?? []).filter(
    (e) => !e.startsWith("Blocked script execution in 'about:srcdoc'"),
  );
  expect(errors).toEqual([]);
});

test("compose, send, undo with z: the draft comes back", async ({ page }) => {
  await onboard(page);
  const c = await writeNew(page, "Notes for Thursday");
  await expect(c.getByRole("button", { name: "Remove Mara Okafor <mara@example.com>" })).toBeVisible();
  await page.keyboard.press("Control+Enter");
  await expect(c).toHaveCount(0);
  await expect(page.getByText("Sending…")).toBeVisible();
  await page.keyboard.press("z");
  await expect(page.getByText("Sending cancelled.")).toBeVisible();
  await expect(composer(page).getByLabel("Subject")).toHaveValue("Notes for Thursday");
  await expect(composer(page).getByLabel("Message")).toHaveValue(/The notes are attached/);
  await page.keyboard.press("Escape");
  await expect(composer(page)).toHaveCount(0);
  await expect(page.getByText("Draft saved.")).toBeVisible();
});

test("undo send off: sent mail shows up in Sent", async ({ page }) => {
  await onboard(page);
  await page.keyboard.press("Control+,");
  await expect(page).toHaveURL(/\/settings$/);
  await page.getByRole("link", { name: "Mail", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Undo send window" }).getByText("Off").click();
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox$/);
  await expect(page.getByRole("listbox", { name: "Inbox conversations" })).toBeVisible();
  await page.getByRole("listbox", { name: "Inbox conversations" }).focus();
  await writeNew(page, "Budget questions");
  await page.keyboard.press("Control+Enter");
  await expect(page.getByText("Sent.", { exact: true })).toBeVisible();
  await page.keyboard.press("g");
  await page.keyboard.press("t");
  await expect(
    page.getByRole("listbox", { name: "Sent conversations" }).getByText("Budget questions"),
  ).toBeVisible();
});

test("reply with r, then send later and cancel it from the palette", async ({ page }) => {
  await onboard(page);
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("article")).toBeVisible();
  await page.keyboard.press("r");
  const c = page.getByRole("dialog", { name: "Reply" });
  await expect(c).toBeVisible();
  await expect(c.getByLabel("Subject")).toHaveValue(/^Re: /);
  await expect(c.getByLabel("Message")).toBeFocused();
  await page.keyboard.type("Sounds good, thanks.");
  await page.keyboard.press("Control+Shift+L");
  const picker = page.getByRole("dialog", { name: "Send later" });
  await expect(picker).toBeVisible();
  await page.keyboard.type("tomorrow 9am");
  await page.keyboard.press("Enter");
  await expect(page.getByText(/^Scheduled for /)).toBeVisible();
  await page.keyboard.press("Control+k");
  await page.getByRole("dialog", { name: "Command palette" }).getByRole("combobox").fill("scheduled");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Scheduled messages" });
  await expect(dialog.getByText(/^Re: /)).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByText("Cancelled. The message stays in Drafts.")).toBeVisible();
});

test("settings: read receipts need the warning, the keymap switches, splits can be added", async ({
  page,
}) => {
  await onboard(page);
  await page.goto("/settings/mail");
  await page.getByRole("switch", { name: "Read receipts" }).click();
  const warning = page.getByRole("alertdialog", { name: "Before you turn on read receipts" });
  await expect(warning).toContainText("Apple Mail Privacy Protection");
  await expect(warning.getByRole("button", { name: "Turn on read receipts" })).toBeDisabled();
  await warning.getByText("I understand").click();
  await warning.getByRole("button", { name: "Turn on read receipts" }).click();
  await expect(page.getByRole("switch", { name: "Read receipts" })).toHaveAttribute("aria-checked", "true");

  await page.getByRole("link", { name: "Shortcuts" }).click();
  await page.getByRole("radiogroup", { name: "Keymap" }).getByText("Gmail").click();
  await expect(
    page.getByRole("radiogroup", { name: "Keymap" }).getByRole("radio", { name: "Gmail" }),
  ).toBeChecked();

  await page.getByRole("link", { name: "Split inbox" }).click();
  await page.getByRole("button", { name: "New split" }).click();
  await page.getByLabel("Name", { exact: true }).fill("Receipts");
  await page.getByLabel("Condition 1 value").fill("shop.example.net");
  await page.getByRole("button", { name: "Save split" }).click();
  await expect(page.getByText("Split “Receipts” created.")).toBeVisible();
  await page.getByRole("link", { name: "Back to mail" }).click();
  await expect(page.getByRole("tab", { name: /Receipts/ })).toBeVisible();
});

test("AI: consent, Akatsuki AI, then a summary with its egress label", async ({ page }) => {
  await onboard(page);
  await page.goto("/settings/ai");
  await page.getByRole("button", { name: "Turn on AI features" }).click();
  await page.getByRole("button", { name: "Use Akatsuki AI" }).click();
  await expect(page.getByText("Akatsuki AI added.")).toBeVisible();
  await expect(page.getByRole("row", { name: /Default/ })).toContainText(
    "Sent to Akatsuki for processing, not stored",
  );
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox$/);
  await page.keyboard.press("Enter");
  await expect(page.getByRole("article")).toBeVisible();
  await expect(page.getByText("Sent to Akatsuki for processing, not stored").first()).toBeVisible();
  await page.keyboard.press("Shift+S");
  await expect(
    page.getByText("They want your review of the attached notes before Friday's meeting."),
  ).toBeVisible();
  await page.keyboard.press("Shift+R");
  await page.getByRole("button", { name: /Sounds good/ }).click();
  await expect(page.getByRole("dialog", { name: "Reply" }).getByLabel("Message")).toHaveValue(
    /I'll take a look/,
  );
});

test("billing shows the Pro trial", async ({ page }) => {
  await onboard(page);
  await page.goto("/settings/billing");
  await expect(page.getByRole("heading", { name: "Pro trial" })).toBeVisible();
  await expect(page.getByText(/\$20 a month/).first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Manage billing" })).toBeVisible();
});
