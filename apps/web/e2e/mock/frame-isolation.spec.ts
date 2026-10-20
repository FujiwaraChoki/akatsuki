// Email markup never paints over the app (§14.3, D-399). The body frame is sandboxed; its height is measured
// from a copy of the HTML in a shadow root of the main document, and that host must not let anything inside
// it paint, whatever the message's CSS says. Checked in a real browser by comparing screenshots.
import { expect, type Page, test } from "@playwright/test";

async function openFirstThread(page: Page) {
  await page.goto("/");
  await page.getByLabel("Email").fill("you@example.com");
  await page.getByRole("button", { name: "Email me a code" }).click();
  await page.getByLabel("Code").fill("123456");
  await page.getByRole("button", { name: "Sign in" }).click();
  await page.getByRole("button", { name: /Gmail/ }).click();
  await expect(page).toHaveURL(/\/inbox$/);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/inbox\/t\/thr\d+$/);
  await expect(page.locator("iframe.ak-message__frame").last()).toBeVisible();
  await expect(page.locator(".ak-measure").first()).toBeAttached();
}

/** A block that tries to escape its host: visible again, pushed back on screen by a huge margin. */
const ESCAPE = `<div style="visibility:visible;margin-left:100000px;width:420px;height:320px;background:#00ff00"></div>`;

const shot = (page: Page) =>
  page.screenshot({ clip: { x: 0, y: 0, width: 420, height: 320 }, animations: "disabled", caret: "hide" });

test("a body that asks to be visible off its measuring host is not painted", async ({ page }) => {
  await openFirstThread(page);
  const before = await shot(page);

  // A second host styled exactly like the app's own (class and inline styles copied from the live one).
  await page.evaluate((html) => {
    const real = document.querySelector<HTMLElement>(".ak-measure")!;
    const host = document.createElement("div");
    host.className = real.className;
    host.setAttribute("style", real.getAttribute("style") ?? "");
    host.id = "probe-host";
    host.attachShadow({ mode: "open" }).innerHTML = html;
    document.body.appendChild(host);
  }, ESCAPE);
  expect((await shot(page)).equals(before)).toBe(true);

  // Control: with only the old styles (hidden, off screen, layout containment) the same block does paint, so the
  // comparison above would have caught the escape.
  await page.evaluate((html) => {
    document.getElementById("probe-host")?.remove();
    const host = document.createElement("div");
    host.id = "control-host";
    host.style.cssText =
      "position:absolute;top:0;left:-100000px;visibility:hidden;pointer-events:none;contain:layout style;width:420px";
    host.attachShadow({ mode: "open" }).innerHTML = html;
    document.body.appendChild(host);
  }, ESCAPE);
  expect((await shot(page)).equals(before)).toBe(false);
});
