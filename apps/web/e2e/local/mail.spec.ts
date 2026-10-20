// Against the LOCAL harness plus a real sync worker and the fake IMAP/SMTP servers, all started by
// `node scripts/e2e-local.mjs --web` (make e2e-web), which passes AKATSUKI_E2E_FAKE_MAIL_STATE and
// AKATSUKI_E2E_SYNC_URL. Skipped otherwise. Flow: email-code sign-in → connect the fake IMAP mailbox through the
// onboarding form → the synced inbox → a hostile HTML message renders sanitized in the sandboxed frame → archive
// with `e` reaches the IMAP server → undo with `z` brings it back on the server and in the list.
import { readFileSync } from "node:fs";
import { expect, type Page, test } from "@playwright/test";
import { readOtp } from "./otp";

type FakeMail = {
  host: string;
  imapPort: number;
  smtpPort: number;
  controlUrl: string;
  email: string;
  username: string;
  password: string;
};
type FakeState = { mailboxes: Record<string, { subject: string }[]> };

const stateFile = process.env.AKATSUKI_E2E_FAKE_MAIL_STATE;

async function mailboxOf(fake: FakeMail, subject: string): Promise<string | null> {
  const s = (await (await fetch(`${fake.controlUrl}/state`)).json()) as FakeState;
  for (const [name, msgs] of Object.entries(s.mailboxes))
    if (msgs.some((m) => m.subject === subject)) return name;
  return null;
}

async function fillEndpoint(page: Page, legend: string, host: string, port: number) {
  await page.getByLabel(`${legend} host`).fill(host);
  await page.getByLabel(`${legend} port`).fill(String(port));
  await page.getByLabel(`${legend} security`).selectOption("implicit");
}

test.skip(!stateFile, "needs scripts/e2e-local.mjs --web (fake mailbox and worker)");

test("sign in, connect IMAP, read a sanitized body, archive with e, undo with z", async ({ page }) => {
  const fake = JSON.parse(readFileSync(stateFile as string, "utf8")) as FakeMail;
  const email = `web-mail-e2e-${Date.now()}-${Math.random().toString(36).slice(2, 8)}@example.com`;
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page).toHaveURL(/\/signin$/);
  await page.getByLabel("Email").fill(email);
  const code = readOtp(email);
  await page.getByRole("button", { name: "Email me a code" }).click();
  await page.getByLabel("Code").fill(await code);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { name: "Bring your inbox." })).toBeVisible({ timeout: 20_000 });

  // Connect the fake mailbox through the IMAP form (the worker tests IMAP LOGIN and SMTP AUTH live).
  await page.getByRole("button", { name: /Other \(IMAP\)/ }).click();
  await expect(page).toHaveURL(/\/onboarding\/imap$/);
  await page.getByLabel("Email address").fill(fake.email);
  await page.getByLabel("Password").fill(fake.password);
  // Server discovery runs on focus and may open the server settings itself; wait for it to settle first.
  await expect(page.getByText(/We guessed these settings|Settings found/)).toBeVisible({ timeout: 15_000 });
  const toggle = page.getByRole("button", { name: /Server settings|Hide server settings/ });
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
  await expect(page.getByLabel("Username")).toBeVisible();
  await page.getByLabel("Username").fill(fake.username);
  await fillEndpoint(page, "Incoming (IMAP)", fake.host, fake.imapPort);
  await fillEndpoint(page, "Outgoing (SMTP)", fake.host, fake.smtpPort);
  await page.getByRole("button", { name: "Connect" }).click();
  await expect(page).toHaveURL(/\/inbox$/, { timeout: 30_000 });

  // The synced inbox: the seeded conversations from the fake server (the API flow may have delivered more).
  const list = page.getByRole("listbox", { name: "Inbox conversations" });
  for (const subject of [/Welcome to the e2e inbox/, /Quarterly report \(HTML\)/, /Lunch on Friday\?/]) {
    await expect(list.getByRole("option", { name: subject })).toBeVisible({ timeout: 60_000 });
  }

  // The hostile HTML message renders sanitized inside the sandboxed frame.
  await list.getByRole("option", { name: /Quarterly report \(HTML\)/ }).click();
  await expect(page).toHaveURL(/\/inbox\/t\//);
  const frame = page.locator("iframe.ak-message__frame").last();
  await expect(frame).toHaveAttribute("sandbox", "allow-popups allow-popups-to-escape-sandbox");
  const body = frame.contentFrame().locator("body");
  await expect(body).toContainText("Numbers are up.", { timeout: 30_000 });
  await expect(frame.contentFrame().locator("script, form, iframe, input")).toHaveCount(0);
  await expect(frame.contentFrame().locator('img[src^="http"]')).toHaveCount(0);
  await expect(frame.contentFrame().locator('a[href^="javascript:"]')).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/inbox$/);

  // Archive the welcome thread with the keyboard; the move reaches the IMAP server.
  const welcome = "Welcome to the e2e inbox";
  await list.getByRole("option", { name: new RegExp(welcome) }).click();
  await expect(page).toHaveURL(/\/inbox\/t\//);
  await page.keyboard.press("e");
  await expect(page.getByText("Archived.")).toBeVisible();
  await expect(list.getByRole("option", { name: new RegExp(welcome) })).toHaveCount(0);
  await expect.poll(() => mailboxOf(fake, welcome), { timeout: 30_000 }).toBe("Archive");

  // Undo with z: back in the list and back in INBOX on the server.
  await page.keyboard.press("z");
  await expect(list.getByRole("option", { name: new RegExp(welcome) })).toHaveCount(1, { timeout: 30_000 });
  await expect.poll(() => mailboxOf(fake, welcome), { timeout: 30_000 }).toBe("INBOX");

  // Clean up through the UI: remove the mailbox (the worker purges it), so runs do not pile up dead accounts.
  await page.goto("/settings/accounts");
  await page.getByRole("button", { name: "Remove mailbox" }).click();
  await page.getByRole("button", { name: `Remove ${fake.email}` }).click();
  await expect(page.getByText(`Removing ${fake.email}.`)).toBeVisible();

  expect(errors).toEqual([]);
});
