import { expect, type Page, test } from "@playwright/test";

// Smoke tests against the static export, served with the vercel.json headers (production CSP), on
// a desktop and a phone project (playwright.config.ts).
const PAGES = [
  { path: "/", h1: /Clear your inbox by first light/ },
  { path: "/pricing", h1: /One plan for the hosted app/ },
  { path: "/download", h1: /Coming to every screen/ },
  { path: "/privacy", h1: /^Privacy$/ },
  { path: "/terms", h1: /^Terms$/ },
  { path: "/license", h1: /^Licence$/ },
  { path: "/security", h1: /^Security$/ },
  { path: "/changelog", h1: /^Changelog$/ },
];

/** Collects console errors, page errors (CSP violations included) and any request off this origin. */
function watch(page: Page) {
  const problems: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error") problems.push(`console: ${m.text()}`);
  });
  page.on("pageerror", (e) => problems.push(`pageerror: ${e.message}`));
  page.on("request", (r) => {
    const url = new URL(r.url());
    if (!["127.0.0.1", "localhost"].includes(url.hostname) && url.protocol !== "data:") {
      problems.push(`third-party request: ${r.url()}`);
    }
  });
  return problems;
}

const overflow = (page: Page) =>
  page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);

for (const p of PAGES) {
  test(`${p.path} renders cleanly under the CSP, with no third-party requests`, async ({ page }) => {
    const problems = watch(page);
    const res = await page.goto(p.path, { waitUntil: "networkidle" });
    expect(res?.status()).toBe(200);
    await expect(page.locator("h1")).toHaveCount(1);
    await expect(page.locator("h1")).toHaveText(p.h1);
    await expect(page).toHaveTitle(/Akatsuki/);
    expect(await page.locator('meta[property="og:image"]').getAttribute("content")).toContain(
      "opengraph-image.png",
    );
    // The logo is the red dot, named for assistive tech, and it goes home.
    const logo = page.getByRole("link", { name: "Akatsuki", exact: true }).first();
    await expect(logo).toHaveAttribute("href", "/");
    await expect(logo.locator(".logo-dot")).toBeVisible();
    expect(await overflow(page)).toBeLessThanOrEqual(0);
    expect(problems).toEqual([]);
  });
}

test("a missing page gets the 404 document", async ({ page }) => {
  const res = await page.goto("/no-such-page");
  expect(res?.status()).toBe(404);
  await expect(page.locator("h1")).toHaveText("Not in this inbox");
});

test("every internal link resolves", async ({ page, request }) => {
  const seen = new Set<string>();
  for (const p of PAGES) {
    await page.goto(p.path);
    const hrefs = await page
      .locator("a[href^='/']")
      .evaluateAll((as) => as.map((a) => a.getAttribute("href") ?? "/"));
    for (const h of hrefs) seen.add(h.split("#")[0] || "/");
  }
  for (const href of seen) {
    const res = await request.get(href);
    expect(res.status(), href).toBe(200);
  }
});

test("SEO files, icons and headers exist", async ({ request }) => {
  const robots = await request.get("/robots.txt");
  expect(await robots.text()).toContain("Sitemap: https://akatsuki.vc/sitemap.xml");
  const sitemap = await request.get("/sitemap.xml");
  expect(await sitemap.text()).toContain("<loc>https://akatsuki.vc/pricing</loc>");
  for (const [path, min] of [
    ["/opengraph-image.png", 10_000],
    ["/twitter-image.png", 10_000],
    ["/icon.png", 200],
    ["/apple-icon.png", 200],
    ["/icon-192.png", 200],
    ["/icon-512.png", 200],
  ] as const) {
    const res = await request.get(path);
    expect(res.headers()["content-type"], path).toBe("image/png");
    expect((await res.body()).length, path).toBeGreaterThan(min);
  }
  const manifest = await (await request.get("/manifest.webmanifest")).json();
  expect(manifest.icons.map((i: { src: string }) => i.src)).toContain("/icon-512.png");
  const csp = (await request.get("/")).headers()["content-security-policy"] ?? "";
  expect(csp).toContain("connect-src 'self'");
  expect(csp).toMatch(/script-src 'self' 'sha256-[A-Za-z0-9+/=]+';/);
  expect(/script-src ([^;]+)/.exec(csp)?.[1]).not.toContain("unsafe-inline");
});

test("legal pages are marked as drafts and the licence is open source, free for personal use", async ({
  page,
}) => {
  await page.goto("/privacy");
  await expect(page.getByText("Draft for lawyer review").first()).toBeVisible();
  await page.goto("/license");
  const text = (await page.locator("main").innerText()).replace(/\s+/g, " ");
  expect(text).toContain("Open source, free for personal use.");
  expect(text).toMatch(/Commercial use[^.]*needs written permission/);
  expect(text).toContain("not an OSI-approved licence");
  // Every other "open source" is the same phrase, or the licence text naming the Open Source Initiative.
  for (const m of text.matchAll(/open source(?! initiative)/gi)) {
    expect(text.slice(m.index).toLowerCase()).toMatch(/^open source, free for personal use/);
  }
});

test("reduced motion: nothing animates and every figure shows its finished state", async ({ browser }) => {
  const ctx = await browser.newContext({ reducedMotion: "reduce" });
  const page = await ctx.newPage();
  await page.goto("/", { waitUntil: "networkidle" });
  await page.mouse.wheel(0, 1600);
  await page.waitForTimeout(400);
  expect(await page.evaluate(() => document.documentElement.classList.contains("js"))).toBe(false);
  expect(
    await page.evaluate(() => document.getAnimations().filter((a) => a.playState === "running").length),
  ).toBe(0);
  // The timeline is fully drawn and the sun has risen.
  await expect(page.locator(".tl-stop.is-done")).toHaveCount(4);
  expect(await page.locator(".sun-disc").evaluate((el) => getComputedStyle(el).transform)).toBe("none");
  await ctx.close();
});

test.describe("desktop", () => {
  test.skip(({ isMobile }) => isMobile, "keyboard interactions are desktop-only");

  test("the command palette opens on Ctrl K and navigates", async ({ page }) => {
    const problems = watch(page);
    await page.goto("/pricing", { waitUntil: "networkidle" });
    await page.keyboard.press("Control+k");
    const dialog = page.getByRole("dialog", { name: "Search the site" });
    await expect(dialog).toBeVisible();
    await page.keyboard.type("down");
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/download$/);
    await page.getByRole("button", { name: "Search" }).first().click();
    await expect(page.getByRole("dialog", { name: "Search the site" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: "Search the site" })).toBeHidden();
    expect(problems).toEqual([]);
  });

  test("the hero inbox answers to the shared key map without a click", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    // Letter keys go to the inbox once it is half in view (live mode); no click, no focus.
    await page.evaluate(() =>
      document.querySelector("[data-frame]")?.scrollIntoView({ block: "center", behavior: "instant" }),
    );
    await expect(page.locator(".ib-live")).toContainText(/live/i);
    const list = page.getByRole("listbox", { name: /Demo inbox/ });
    const options = list.getByRole("option");
    const before = await options.count();
    await page.keyboard.press("j");
    await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("e");
    await expect(options).toHaveCount(before - 1);
    await expect(page.locator("[data-status]")).toContainText("Archived");
    await page.keyboard.press("z");
    await expect(options).toHaveCount(before);
    // Space still scrolls the page: live mode never captures it.
    const y = await page.evaluate(() => scrollY);
    await page.keyboard.press("Space");
    await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(y);
  });

  test("the inbox never traps Tab or Escape (WCAG 2.1.2)", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const list = page.getByRole("listbox", { name: /Demo inbox/ });
    await list.focus();
    await page.keyboard.press("Escape");
    await expect(list).not.toBeFocused();
    await list.focus();
    await page.keyboard.press("Tab");
    await expect(list).not.toBeFocused();
  });

  test("the quiet keyboard lights the pressed key and names its command", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#keys").scrollIntoViewIfNeeded();
    await page.locator(".km-tray").scrollIntoViewIfNeeded();
    const verb = page.locator("[data-km-verb]");
    await expect(verb).toHaveText("Archive");
    for (const [key, name] of [
      ["h", "Snooze"],
      ["Shift+H", "Remind me"],
      ["/", "Search"],
      ["e", "Archive"],
    ] as const) {
      await page.keyboard.press(key);
      await expect(verb).toHaveText(name);
    }
    await expect(page.locator(".km-board .km-key.is-lit")).toHaveCount(1);
    await expect(page.locator(".km-board .km-key.is-lit")).toHaveAttribute("data-k", "e");
    await page.locator(".km-board [data-k='r']").click();
    await expect(verb).toHaveText("Reply");
  });

  test("the theme control switches and remembers the theme", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const ctl = page.locator("footer [data-theme-ctl]").first();
    await ctl.getByRole("button", { name: "Dark" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await page.reload();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe(
      "rgb(10, 10, 10)",
    );
    await page.locator("footer [data-theme-ctl]").first().getByRole("button", { name: "Light" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    expect(await page.evaluate(() => getComputedStyle(document.body).backgroundColor)).toBe(
      "rgb(255, 255, 255)",
    );
  });

  test("the waitlist checks the address and posts nothing", async ({ page }) => {
    const posted: string[] = [];
    page.on("request", (r) => {
      if (r.method() === "POST") posted.push(r.url());
    });
    await page.goto("/", { waitUntil: "networkidle" });
    const form = page.getByRole("form", { name: "Join the waitlist by email" });
    await expect(form).toHaveAttribute("action", /^mailto:sami@samihindi\.com\?subject=Akatsuki%20waitlist/);
    await page.getByRole("textbox", { name: "From" }).fill("me@");
    await form.getByRole("button", { name: /Email us to join/ }).click();
    await expect(page.getByRole("alert")).toHaveText("That address looks incomplete.");
    expect(posted).toEqual([]);
  });
});

test.describe("phone", () => {
  test.skip(({ isMobile }) => !isMobile, "phone layout");

  test("gets a menu, a specimen list instead of the keyboard, and no overlap or overflow", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.waitForTimeout(2600); // the inbox's one entrance
    await expect(page.locator(".km-tray")).toBeHidden();
    await expect(page.locator(".km-spec")).toHaveCount(6);
    await page.locator(".km-spec[data-k='h']").tap();
    await expect(page.locator(".km-spec.is-lit .km-verb")).toHaveText(["Snooze"]);
    const gap = await page.evaluate(() => {
      const frame = document.querySelector("[data-frame]")?.getBoundingClientRect();
      const bar = document.querySelector(".touchbar")?.getBoundingClientRect();
      return frame && bar ? bar.top - frame.bottom : -1;
    });
    expect(gap).toBeGreaterThanOrEqual(0);
    for (const w of [320, 360, 390]) {
      await page.setViewportSize({ width: w, height: 800 });
      expect(await overflow(page), `${w}px`).toBeLessThanOrEqual(0);
    }
    await page.getByRole("button", { name: "Menu" }).click();
    await expect(page.getByRole("dialog", { name: "Menu" })).toBeVisible();
  });

  test("the touch bar archives the selected conversation", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const options = page.getByRole("listbox", { name: /Demo inbox/ }).getByRole("option");
    const before = await options.count();
    await page.locator("[data-touch='e']").tap();
    await expect(options).toHaveCount(before - 1);
    await page.locator("[data-touch='z']").tap();
    await expect(options).toHaveCount(before);
  });
});
