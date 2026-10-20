// Review round 3 on the production bundle (mock backend, production CSP): focus comes back to the draft after the
// palette, the search field follows the list on screen, labels can be created from the picker and renamed or
// deleted in Settings, and phone toasts stay clear of the top controls.
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

const inbox = (page: Page) => page.getByRole("listbox", { name: "Inbox conversations" });
const composer = (page: Page) => page.getByRole("dialog", { name: /New message|Reply|Forward/ });

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

test("the palette gives the keyboard back to the draft: typing never runs list shortcuts", async ({
  page,
}) => {
  await onboard(page);
  const rows = await inbox(page).getByRole("option").count();
  await page.keyboard.press("c");
  const c = composer(page);
  await expect(c.getByLabel("To", { exact: true })).toBeFocused();
  await c.getByLabel("Message").click();
  await page.keyboard.type("Hello");
  await page.keyboard.press("Control+k");
  await expect(page.getByRole("combobox", { name: "Command palette" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(c.getByLabel("Message")).toBeFocused();
  await page.keyboard.type("e#a");
  await expect(c.getByLabel("Message")).toHaveValue("Helloe#a");
  await expect(page.getByText("Archived.")).toHaveCount(0);
  await expect(inbox(page).getByRole("option")).toHaveCount(rows);

  // A compose command from the palette focuses the field it acts on.
  await page.keyboard.press("Control+k");
  await page.keyboard.type("Add Cc");
  await page.keyboard.press("Enter");
  await expect(c.getByLabel("Cc", { exact: true })).toBeFocused();
  await page.keyboard.type("lena");
  await expect(c.getByLabel("Cc", { exact: true })).toHaveValue("lena");
  await expect(page.getByRole("dialog", { name: "Labels" })).toHaveCount(0);
});

test("the search field is cleared when the list on screen is not a search", async ({ page }) => {
  await onboard(page);
  const search = page.getByRole("combobox", { name: "Search mail" });
  await page.keyboard.press("/");
  await expect(search).toBeFocused();
  await page.keyboard.type("from:lena");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/search/);
  await expect(search).toHaveValue("from:lena");
  // Escape from the list goes back to the inbox: the field is empty again.
  await search.focus();
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox$/);
  await expect(search).toHaveValue("");
  // g s to Starred after another search: empty too.
  await search.focus();
  await page.keyboard.type("from:lena");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/search/);
  await inbox(page)
    .or(page.getByRole("listbox", { name: /conversations/ }))
    .first()
    .focus();
  await page.keyboard.press("g");
  await page.keyboard.press("s");
  await expect(page).toHaveURL(/\/starred$/);
  await expect(search).toHaveValue("");
  // The last search is one keystroke away.
  await search.focus();
  await expect(page.getByRole("option", { name: /from:lena/ })).toBeVisible();
});

test("create a label from the picker, then rename and delete it in Settings", async ({ page }) => {
  await onboard(page);
  await inbox(page).focus();
  await page.keyboard.press("l");
  const picker = page.getByRole("dialog", { name: "Labels" });
  await expect(picker).toBeVisible();
  await page.keyboard.type("Projects");
  await expect(picker.getByRole("option", { name: "Create label “Projects”" })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.getByText("Created and added Projects.")).toBeVisible();
  await expect(page.getByRole("navigation").getByText("Projects")).toBeVisible();

  await page.keyboard.press("Control+k");
  await page.keyboard.type("Settings: Labels");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/settings\/labels$/);
  await page.getByRole("button", { name: "Rename Projects" }).click();
  const field = page.getByLabel("New name for Projects");
  await field.fill("Projects 2026");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Renamed to “Projects 2026”.")).toBeVisible();
  await page.getByRole("button", { name: "Delete Projects 2026" }).click();
  await page.getByRole("button", { name: "Delete Projects 2026" }).click();
  await expect(page.getByText("Deleted “Projects 2026”.")).toBeVisible();
  await expect(page.getByRole("button", { name: /Rename Projects/ })).toHaveCount(0);
});

test("phones: toasts dock at the bottom, clear of the search field and a thread's toolbar", async ({
  page,
}) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await onboard(page);
  const clear = async (toastText: string | RegExp, other: ReturnType<Page["locator"]>) => {
    const toast = page.locator(".ak-toast").filter({ hasText: toastText }).first();
    await expect(toast).toBeVisible();
    const box = (await toast.boundingBox())!;
    // Docked at the bottom (several toasts stack upwards from there).
    expect(box.y).toBeGreaterThan(812 / 2);
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(375);
    const b = (await other.boundingBox())!;
    expect(b.y + b.height <= box.y || b.y >= box.y + box.height).toBe(true);
  };
  // In the list: the toast leaves the search field and the list title alone.
  await inbox(page).focus();
  await page.keyboard.press("e");
  await clear("Archived.", page.getByRole("combobox", { name: "Search mail" }));
  // In a thread: the toolbar (archive, trash, snooze, star, labels) stays reachable.
  await inbox(page).focus();
  await page.keyboard.press("Enter");
  const toolbar = page.locator(".ak-thread__toolbar");
  await expect(toolbar).toBeVisible();
  await page.keyboard.press("s");
  await clear(/Starred|Unstarred/, toolbar);
});
