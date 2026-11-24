// The page contract (docs/tool-api.md, Logix Viewer): both pages ask their
// host through window.logixHost, the one way for a host's own window (the
// user's decision, 2026-11-22).
import { test, expect } from "@playwright/test";
import { open, row, hostCalls, hostScript, showSignal, PAGES, VIEWER_FEATURES } from "./helpers";
import * as fs from "node:fs";
import * as path from "node:path";

test("the waveform page saves the view with writeDeclared, as the .view file's text", async ({ page }) => {
  await open(page, "plain");
  await row(page, "led_on").click({ button: "right" });
  await page.locator('.ctx [data-color="3"]').click();
  await expect.poll(async () => (await hostCalls(page)).filter((c) => c[0] === "writeDeclared").length).toBeGreaterThan(0);
  const [, id, text] = (await hostCalls(page)).filter((c) => c[0] === "writeDeclared").pop()!;
  expect(id).toBe("view");
  expect((text as string).endsWith("\n")).toBe(true);
  expect(JSON.parse(text as string).version).toBe(1);
});

test("the waveform page opens a signal's declaration with openSource", async ({ page }) => {
  await open(page, "plain");
  await row(page, "led_on").dblclick();
  const call = (await hostCalls(page)).find((c) => c[0] === "openSource");
  expect(call?.[1]).toBe("morse_tb.v");
  expect(Number(call?.[2])).toBeGreaterThan(0);
});

test("double-clicking a signal's wave opens its declaration too", async ({ page }) => {
  await open(page, "plain");
  const r = (await row(page, "led_on").boundingBox())!;
  const w = (await page.locator("#waves").boundingBox())!;
  await page.mouse.dblclick(w.x + w.width / 2, r.y + r.height / 2);
  const call = (await hostCalls(page)).find((c) => c[0] === "openSource");
  expect(call?.[1]).toBe("morse_tb.v");
});

test("double-clicking a design signal says its place isn't known, and opens nothing", async ({ page }) => {
  await open(page, "plain");
  await showSignal(page, "dut", "pattern");
  await row(page, "pattern").dblclick();
  await expect(page.locator("#toast")).toContainText("isn't known");
  expect((await hostCalls(page)).some((c) => c[0] === "openSource")).toBe(false);
});

test("a waveform opened without its testbench says so on a double-click, and opens nothing", async ({ page }) => {
  await open(page, "no-testbench");
  await row(page, "led_on").dblclick();
  await expect(page.locator("#toast")).toContainText("opened without its testbench");
  expect((await hostCalls(page)).some((c) => c[0] === "openSource")).toBe(false);
});

test("the help list fits a short window and scrolls to its last line", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 320 });
  await open(page, "plain");
  await page.locator("#helpBtn").click();
  const pop = page.locator(".pop");
  const box = (await pop.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(320);
  const last = pop.locator(".keys span").last();
  await last.scrollIntoViewIfNeeded();
  await expect(last).toBeInViewport();
  expect(await pop.evaluate((e) => e.scrollHeight > e.clientHeight)).toBe(true);
});

test("a testbench signal's place is in its hover boxes", async ({ page }) => {
  await open(page, "plain", VIEWER_FEATURES);
  expect(await row(page, "led_on").getAttribute("title")).toMatch(/^morse_tb\.led_on\nDeclared in morse_tb\.v, line \d+$/);
  const r = (await row(page, "led_on").boundingBox())!;
  const w = (await page.locator("#waves").boundingBox())!;
  await page.mouse.move(w.x + w.width / 2, r.y + r.height / 2);
  await expect(page.locator("#tip")).toContainText("Declared in morse_tb.v, line");
  // A design signal has no known place: its hover box has none.
  await showSignal(page, "dut", "pattern");
  expect(await row(page, "pattern").getAttribute("title")).toBe("morse_tb.dut.pattern");
});

test("in Logix Viewer, with no openSource, double-clicking a signal opens nothing and shows no message", async ({ page }) => {
  await open(page, "plain", VIEWER_FEATURES);
  await row(page, "led_on").dblclick();
  await showSignal(page, "dut", "pattern");
  await row(page, "pattern").dblclick();
  await expect(page.locator("#toast")).toBeHidden();
  expect((await hostCalls(page)).some((c) => c[0] === "openSource")).toBe(false);
});

test("the waveform page saves an image with saveAs", async ({ page }) => {
  await open(page, "plain");
  await page.locator("#imageBtn").click();
  await page.getByRole("menuitem", { name: /SVG/ }).first().click();
  await expect.poll(async () => (await hostCalls(page)).some((c) => c[0] === "saveAs")).toBe(true);
  const [, name, mime] = (await hostCalls(page)).find((c) => c[0] === "saveAs")!;
  expect(name).toMatch(/\.svg$/);
  expect(mime).toBe("image/svg+xml");
});

test("the diagram page opens a block's source with openSource, and saves images with saveAs", async ({ page }) => {
  const dir = path.join(PAGES, "diagram");
  const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript(hostScript);
  await page.goto("file://" + path.join(dir, file) + "?theme=light");
  // Hosted: the module's "Go to source" shows, and goes through the host.
  await page.locator("#source").click();
  const open_ = (await hostCalls(page)).find((c) => c[0] === "openSource");
  expect(open_?.[1]).toBe("main.v");
  await page.getByRole("button", { name: /Export/ }).click();
  await page.getByRole("menuitem", { name: /SVG/ }).first().click();
  await expect.poll(async () => (await hostCalls(page)).some((c) => c[0] === "saveAs")).toBe(true);
  // Plain colors, no CSS variables: older librsvg, as Ubuntu 22.04's image
  // viewer uses, drops var() rules and showed a blank picture (2026-11-23),
  const [, , mime, svg] = (await hostCalls(page)).find((c) => c[0] === "saveAs")!;
  expect(mime).toBe("image/svg+xml");
  // and each shape's paint as attributes, with no stylesheet: Inkscape
  // showed a blank canvas on macOS too.
  expect(svg as string).not.toContain("var(");
  expect(svg as string).not.toContain("<style");
  expect(svg as string).toMatch(/<rect[^>]*class="body"[^>]*fill="rgb\(/);
  // As big as on screen: it fills the drawing's box in one direction
  // (apps showed it small at its units' size, 2026-11-23).
  const box = (await page.locator("svg.logix-diagram").boundingBox())!;
  const [w, h] = [/ width="(\d+)"/, / height="(\d+)"/].map((r) => Number((svg as string).match(r)![1]));
  expect(Math.abs(w - box.width) <= 2 || Math.abs(h - box.height) <= 2).toBe(true);
  expect(errors).toEqual([]);
});

test("in Logix Viewer, with no openSource, the diagram offers no source: its places are in the hover boxes", async ({ page }) => {
  const dir = path.join(PAGES, "diagram");
  const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
  await page.addInitScript(hostScript, VIEWER_FEATURES);
  await page.goto("file://" + path.join(dir, file) + "?theme=light");
  await expect(page.locator("svg.logix-diagram")).toBeVisible();
  await expect(page.locator("#source")).toBeHidden();
  await page.locator("#helpBtn").click();
  await expect(page.locator(".help")).toBeVisible();
  await expect(page.locator(".help")).not.toContainText("Double-click a block");
  expect((await hostCalls(page)).some((c) => c[0] === "openSource")).toBe(false);
});
