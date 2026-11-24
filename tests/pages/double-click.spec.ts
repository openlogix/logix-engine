// A double-click (the user's macOS tests, 2026-11-23 and 24): on a wave or
// a signal's name it opens the declaration where the host opens sources,
// and otherwise it's a click, whose cursor and selection stay; on the
// diagram it selects no text. And the help lists never scroll sideways, at
// any window width.
import { test, expect, type Page } from "@playwright/test";
import { open, row, hostCalls, PAGES, VIEWER_FEATURES } from "./helpers";
import * as fs from "node:fs";
import * as path from "node:path";

/** The cursor in the last view the page saved to its host. */
async function savedCursor(page: Page) {
  const writes = (await hostCalls(page)).filter((c) => c[0] === "writeDeclared");
  return writes.length ? JSON.parse(writes[writes.length - 1][2] as string).cursor : undefined;
}
const selected = (page: Page) => page.locator("#names .nrow.sel").evaluateAll((ns) => ns.map((n) => (n as HTMLElement).dataset.i));

/** A point on led_on's wave, at a share of the wave column's width. */
async function onWave(page: Page, share: number) {
  const r = (await row(page, "led_on").boundingBox())!;
  const w = (await page.locator("#waves").boundingBox())!;
  return { x: w.x + w.width * share, y: r.y + r.height / 2 };
}

for (const [where, features] of [["Logix Viewer (no openSource)", VIEWER_FEATURES], ["a host that opens sources", undefined]] as const) {
  test(`in ${where}, double-clicking a wave acts as a click there`, async ({ page }) => {
    await open(page, "plain", features as string[] | undefined);
    // A single click puts the cursor there and selects the row.
    const a = await onWave(page, 0.3);
    await page.mouse.click(a.x, a.y);
    await expect.poll(() => savedCursor(page)).not.toBeNull();
    const first = await savedCursor(page);
    // A double-click elsewhere, on another row: the cursor goes there, and
    // that row is selected, as a click would, and they stay.
    await page.waitForTimeout(600);   // past the double-click interval
    const other = page.locator("#names .nrow:not(.bit)").nth(0);
    const o = (await other.boundingBox())!;
    const b = await onWave(page, 0.7);
    await page.mouse.dblclick(b.x, o.y + o.height / 2);
    await expect.poll(() => savedCursor(page)).toBeGreaterThan(first);
    await page.waitForTimeout(300);
    expect(await savedCursor(page)).toBeGreaterThan(first);
    expect(await selected(page)).toEqual([await other.getAttribute("data-i")]);
    const opened = (await hostCalls(page)).some((c) => c[0] === "openSource");
    expect(opened).toBe(features === undefined);
  });

  test(`in ${where}, double-clicking a signal's name selects it, as a click`, async ({ page }) => {
    await open(page, "plain", features as string[] | undefined);
    await page.locator("#names .nrow:not(.bit):not(.group)").first().click();
    await page.waitForTimeout(600);
    await row(page, "led_on").dblclick();
    await page.waitForTimeout(100);
    expect(await selected(page)).toEqual([await row(page, "led_on").getAttribute("data-i")]);
    expect((await hostCalls(page)).some((c) => c[0] === "openSource")).toBe(features === undefined);
  });
}

test("a single click on a wave still moves the cursor, and two clicks apart move it twice", async ({ page }) => {
  await open(page, "plain", VIEWER_FEATURES);
  const a = await onWave(page, 0.3), b = await onWave(page, 0.7);
  await page.mouse.click(a.x, a.y);
  await expect.poll(() => savedCursor(page)).not.toBeNull();
  const first = await savedCursor(page);
  await page.waitForTimeout(600);   // past the double-click interval
  await page.mouse.click(b.x, b.y);
  await expect.poll(() => savedCursor(page)).toBeGreaterThan(first);
});

/** The help box: no sideways scroll, inside the window, its labels and descriptions readable. */
async function checkHelp(page: Page, box: ReturnType<Page["locator"]>, width: number) {
  await expect(box).toBeVisible();
  const m = await box.evaluate((e) => {
    const r = e.getBoundingClientRect();
    const spans = [...e.querySelectorAll(".keys span")].map((s) => (s as HTMLElement).getBoundingClientRect().width);
    return { sw: e.scrollWidth, cw: e.clientWidth, left: r.left, right: r.right, width: r.width, narrowest: Math.min(...spans) };
  });
  expect(m.sw).toBeLessThanOrEqual(m.cw);
  expect(m.left).toBeGreaterThanOrEqual(0);
  expect(m.right).toBeLessThanOrEqual(width);
  // Descriptions get real room, not a few characters.
  expect(m.narrowest).toBeGreaterThan(Math.min(150, m.width * 0.4));
  return m;
}

for (const width of [1400, 600, 360]) {
  test(`the waveform help is wide when it can be and never scrolls sideways (${width} px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await open(page, "plain", VIEWER_FEATURES);
    await page.locator("#helpBtn").click();
    const m = await checkHelp(page, page.locator(".pop"), width);
    expect(m.width).toBeCloseTo(Math.min(440, width - 16), 0);
  });

  test(`the diagram help is wide when it can be and never scrolls sideways (${width} px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    const dir = path.join(PAGES, "diagram");
    const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
    await page.goto("file://" + path.join(dir, file) + "?theme=light");
    await expect(page.locator("svg.logix-diagram").first()).toBeVisible();
    await page.locator("#helpBtn").click();
    const m = await checkHelp(page, page.locator(".help"), width);
    expect(m.width).toBeCloseTo(Math.min(440, width - 16), 0);
  });
}

test("double-clicking a diagram block selects no text", async ({ page }) => {
  const dir = path.join(PAGES, "diagram");
  const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
  await page.goto("file://" + path.join(dir, file) + "?theme=light");
  await expect(page.locator("svg.logix-diagram").first()).toBeVisible();
  // A block that isn't a module, on one of its port names if it has one.
  const label = page.locator("svg.logix-diagram g[data-block]:not(.cat-module) text").first();
  const target = (await label.count()) ? label : page.locator("svg.logix-diagram g[data-block]:not(.cat-module)").first();
  const b = (await target.boundingBox())!;
  await page.mouse.dblclick(b.x + b.width / 2, b.y + b.height / 2);
  expect(await page.evaluate(() => String(window.getSelection()))).toBe("");
  // Three clicks, which select a line in plain text, select none either.
  await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2, { clickCount: 3 });
  expect(await page.evaluate(() => String(window.getSelection()))).toBe("");
});
