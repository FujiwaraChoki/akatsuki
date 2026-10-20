// Smoke tests of the production bundle (mock backend) under the production CSP.
import { expect, type Page, test } from "@playwright/test";

const CODE = "123456";

async function signIn(page: Page) {
  await page.goto("/");
  await expect(page).toHaveURL(/\/signin$/);
  await page.getByLabel("Email").fill("you@example.com");
  await page.getByRole("button", { name: "Email me a code" }).click();
  await page.getByLabel("Code").fill(CODE);
  await page.getByRole("button", { name: "Sign in" }).click();
}

async function onboardWithGmail(page: Page) {
  await signIn(page);
  await expect(page.getByRole("heading", { name: "Bring your inbox." })).toBeVisible();
  await page.getByRole("button", { name: /Gmail/ }).click();
  await expect(page).toHaveURL(/\/inbox$/);
  await expect(page.getByRole("listbox", { name: "Inbox conversations" })).toBeVisible();
}

const activeId = (page: Page) =>
  page.getByRole("listbox", { name: /conversations/ }).getAttribute("aria-activedescendant");

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  (page as Page & { errors?: string[] }).errors = errors;
});

test.afterEach(async ({ page }) => {
  // No CSP violations or uncaught errors on any page. Chromium reports the sandbox refusing Playwright's own
  // frame instrumentation in the body iframes ("Blocked script execution in 'about:srcdoc'"): that is the
  // sandbox working, not an app error.
  const errors = ((page as Page & { errors?: string[] }).errors ?? []).filter(
    (e) => !e.startsWith("Blocked script execution in 'about:srcdoc'"),
  );
  expect(errors).toEqual([]);
});

test("sign-in page renders under the production CSP", async ({ page }) => {
  const res = await page.goto("/signin");
  expect(res?.headers()["content-security-policy"]).toContain("default-src 'self'");
  await expect(page.getByRole("heading", { name: "Mail at the speed of thought." })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue with Google" })).toBeVisible();
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", /./);
});

test("sign in, connect Gmail, read and archive with the keyboard, undo with z", async ({ page }) => {
  await onboardWithGmail(page);
  const list = page.getByRole("listbox", { name: "Inbox conversations" });
  const first = await activeId(page);
  await page.keyboard.press("j");
  const second = await activeId(page);
  expect(second).not.toBe(first);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/inbox\/t\/thr\d+$/);
  const frame = page.locator("iframe.ak-message__frame").last();
  await expect(frame).toHaveAttribute("sandbox", "allow-popups allow-popups-to-escape-sandbox");
  await expect(frame.contentFrame().locator("body")).toContainText("Best,");
  // Archive the open thread: it leaves the inbox and the next one opens (auto-advance).
  const openUrl = page.url();
  await page.keyboard.press("e");
  await expect(page.getByText("Archived.")).toBeVisible();
  await expect(page).not.toHaveURL(openUrl);
  await expect(list.locator(`#${second}`)).toHaveCount(0);
  await page.keyboard.press("z");
  await expect(list.locator(`#${second}`)).toHaveCount(1);
  await expect(page).toHaveURL(openUrl);
});

test("keyboard focus: the list has it, F6 cycles landmarks, Tab leaves other controls alone", async ({
  page,
}) => {
  await onboardWithGmail(page);
  const list = page.getByRole("listbox", { name: "Inbox conversations" });
  await expect(list).toBeFocused();
  await page.keyboard.press("F6");
  await expect(page.locator(".ak-reading")).toBeFocused();
  await page.keyboard.press("F6");
  await expect(page.getByRole("navigation", { name: "Mailboxes" }).locator(":focus")).toHaveCount(1);
  // Tab on a sidebar control moves focus, it does not switch splits.
  await page.keyboard.press("Tab");
  await expect(page).toHaveURL(/\/inbox$/);
  await page.keyboard.press("F6");
  await expect(page.getByRole("combobox", { name: "Search mail" })).toBeFocused();
  await page.keyboard.press("F6");
  await expect(list).toBeFocused();
  // Closing the palette gives the keyboard back to the list.
  await page.keyboard.press("Control+k");
  await page.keyboard.press("Escape");
  await expect(list).toBeFocused();
});

test("command palette, theme and shortcuts help", async ({ page }) => {
  await onboardWithGmail(page);
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await palette.getByRole("combobox").fill("dark theme");
  await page.keyboard.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  // The mail shell is its own chunk (D-409): keys work once the list is there.
  await expect(page.getByRole("listbox", { name: /Inbox conversations/ })).toBeFocused();
  await page.keyboard.press("?");
  await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toBeVisible();
});

test("search with suggestions, then the full mailbox", async ({ page }) => {
  await onboardWithGmail(page);
  await page.keyboard.press("/");
  const input = page.getByRole("combobox", { name: "Search mail" });
  await expect(input).toBeFocused();
  await input.fill("is:sta");
  await expect(page.getByRole("option", { name: /is:starred/ })).toBeVisible();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/search\?q=is%3Astarred/);
  await expect(page.getByRole("listbox", { name: "Search conversations" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox$/);
});

test("split tabs, g sequences and Inbox Zero", async ({ page }) => {
  await onboardWithGmail(page);
  await page.keyboard.press("Tab");
  await expect(page).toHaveURL(/\/inbox\/split\/splitvip$/);
  await page.keyboard.press("g");
  await page.keyboard.press("s");
  await expect(page).toHaveURL(/\/starred$/);
  // Empty the News split: select everything, archive.
  await page.goto("/inbox/split/splitnews");
  await expect(page.getByRole("listbox", { name: "Inbox conversations" })).toBeVisible();
  await page.keyboard.press("Control+a");
  await page.keyboard.press("e");
  await expect(page.getByRole("heading", { name: "Inbox zero" })).toBeVisible();
});

test("an op failure shows a toast with Retry, and the thread comes back", async ({ page }) => {
  await onboardWithGmail(page);
  const list = page.getByRole("listbox", { name: "Inbox conversations" });
  const failing = list.getByRole("option", { name: /Shared folder moved/ });
  await expect(failing).toBeVisible();
  await page.keyboard.press("e");
  const alert = page.getByRole("alert").filter({ hasText: "Couldn't archive" });
  await expect(alert).toContainText("in you@…");
  await expect(failing).toBeVisible();
  await alert.getByRole("button", { name: "Retry" }).click();
  await expect(failing).toHaveCount(0);
});

test("remote images stay blocked until asked", async ({ page }) => {
  await onboardWithGmail(page);
  await page.keyboard.press("j");
  await page.keyboard.press("Enter");
  const banner = page.getByRole("note").filter({ hasText: "hidden to protect your privacy" }).first();
  await expect(banner).toBeVisible();
  const frame = page.locator("iframe.ak-message__frame").last();
  await expect(frame).not.toHaveAttribute("srcdoc", /<img[^>]+ src=/);
  await banner.getByRole("button", { name: /Show images/ }).click();
  await expect(page.locator("iframe.ak-message__frame").last()).toHaveAttribute(
    "srcdoc",
    /<img[^>]+ src="data:image/,
  );
});

test("sign out returns to the sign-in page", async ({ page }) => {
  await onboardWithGmail(page);
  // First Light: sign out lives in the account menu ("All inboxes ⌄").
  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { name: "Mail at the speed of thought." })).toBeVisible();
});

test("a phone-width screen has a menu, compose, palette and a way back (375 px)", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  await onboardWithGmail(page);
  const menu = page.getByRole("button", { name: "Menu" });
  await expect(menu).toBeVisible();
  await expect(page.getByRole("button", { name: "Compose" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Command palette" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  // Two-line rows: the subject gets the full width.
  const subject = page.locator(".ak-threadrow__subject").first();
  expect((await subject.boundingBox())!.width).toBeGreaterThan(200);
  // The drawer: mailboxes and settings, closed again by Escape.
  await menu.click();
  const nav = page.getByRole("navigation", { name: "Mailboxes" });
  await expect(nav.getByRole("link", { name: /Starred/ })).toBeVisible();
  await expect(nav.getByRole("button", { name: "Settings" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(nav.getByRole("link", { name: /Starred/ })).toBeHidden();
  // A tapped conversation has a back control.
  await page.locator(".ak-threadrow").nth(1).click();
  await expect(page).toHaveURL(/\/inbox\/t\/thr\d+$/);
  await page.getByRole("button", { name: /Back to Inbox/ }).click();
  await expect(page).toHaveURL(/\/inbox$/);
});
