// Against the LOCAL Convex harness (AKATSUKI_E2E_LOCAL=1): real Better Auth email-OTP sign-in, the user rows
// (trigger or me.bootstrap), onboarding, the empty mail shell, error copy when the sync worker is down, sign-out.
// The harness runs EMAIL_TRANSPORT=console, so the code is read from the function logs (./otp.ts).
import { expect, test } from "@playwright/test";
import { readOtp } from "./otp";

test("email code sign-in, onboarding, empty inbox, worker errors, sign-out", async ({ page }) => {
  const email = `web-e2e-${Date.now()}-${Math.random().toString(36).slice(2, 8)}@example.com`;
  page.on("console", (m) => console.info("[console]", m.type(), m.text().slice(0, 300)));
  page.on("requestfailed", (r) => console.info("[failed]", r.method(), r.url(), r.failure()?.errorText));
  page.on("response", (r) => {
    if (r.status() >= 400) console.info("[http]", r.status(), r.request().method(), r.url());
  });
  await page.goto("/");
  await expect(page).toHaveURL(/\/signin$/);
  await page.getByLabel("Email").fill(email);
  const code = readOtp(email);
  await page.getByRole("button", { name: "Email me a code" }).click();
  await expect(page.getByLabel("Code")).toBeVisible();
  await page.getByLabel("Code").fill(await code);
  await page.getByRole("button", { name: "Sign in" }).click();

  // A new user (billing off on the harness) lands on "connect a mailbox".
  await expect(page.getByRole("heading", { name: "Bring your inbox." })).toBeVisible({ timeout: 20_000 });
  await expect(page).toHaveURL(/\/onboarding$/);

  // Without a sync worker (or with one that has no Google client, as under `make e2e-web`): the error copy of
  // §15, not a crash.
  await page.getByRole("button", { name: /Gmail/ }).click();
  await expect(page.getByRole("alert")).toContainText(/could not be reached|not available/);

  await page.getByRole("button", { name: "I'll do this later" }).click();
  await expect(page).toHaveURL(/\/inbox$/);
  await expect(page.getByRole("heading", { name: "No mailbox yet" })).toBeVisible();

  // Live data: theme preference round-trips through me.updatePrefs and survives a reload.
  await page.keyboard.press("Control+k");
  await page.getByRole("dialog", { name: "Command palette" }).getByRole("combobox").fill("dark theme");
  await page.keyboard.press("Enter");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  // Reload at once, possibly before the save reached the server: the unsaved choice must win over the older
  // server value and be sent again (lib/theme.ts, D-400).
  await page.reload();
  await expect(page.getByRole("heading", { name: "No mailbox yet" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  // Once the server has confirmed it, nothing is pending and a fresh load reads "dark" from the server.
  await page.waitForFunction(() => localStorage.getItem("ak:prefs-pending") === null, undefined, {
    timeout: 15_000,
  });
  await page.evaluate(() => localStorage.removeItem("ak:theme"));
  await page.reload();
  await expect(page.getByRole("heading", { name: "No mailbox yet" })).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  await page.getByRole("button", { name: /account menu/ }).click();
  await page.getByRole("menuitem", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/signin$/);
  await expect(page.getByRole("heading", { name: "Mail at the speed of thought." })).toBeVisible();
});
