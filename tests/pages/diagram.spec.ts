// The diagram page opens, draws the design and offers Export.
import { test, expect } from "@playwright/test";
import { PAGES, hostScript, hostCalls } from "./helpers";
import * as os from "node:os";
import * as fs from "node:fs";
import * as path from "node:path";

test("the diagram page draws the design without errors", async ({ page }) => {
  const dir = path.join(PAGES, "diagram");
  const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("file://" + path.join(dir, file) + "?theme=light");
  await expect(page.locator("svg.logix-diagram").first()).toBeVisible();
  await expect(page.getByRole("button", { name: /Export/ })).toBeVisible();
  expect(errors).toEqual([]);
});

test("Export saves the design as a web page that opens at the zoom shown", async ({ page }) => {
  // The user's request, 2026-11-23: a live page, as the waveform page's.
  const dir = path.join(PAGES, "diagram");
  const file = fs.readdirSync(dir).find((f) => f.endsWith(".html"))!;
  await page.addInitScript(hostScript);
  await page.goto("file://" + path.join(dir, file) + "?theme=light");
  await expect(page.locator("svg.logix-diagram").first()).toBeVisible();
  await page.locator("#zoomIn").click();
  await page.locator("#zoomIn").click();
  const shown = await page.locator("svg.logix-diagram").getAttribute("viewBox");
  await page.getByRole("button", { name: /Export/ }).click();
  await page.getByRole("menuitem", { name: /Web page/ }).click();
  await expect.poll(async () => (await hostCalls(page)).some((c) => c[0] === "saveAs")).toBe(true);
  const [, name, mime, html] = (await hostCalls(page)).find((c) => c[0] === "saveAs")!;
  expect(name).toMatch(/^diagram-.*\.html$/);
  expect(mime).toBe("text/html");

  // The saved page, in a plain browser: the same zoom, no errors.
  const saved = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "diagram-page-")), name as string);
  fs.writeFileSync(saved, html as string);
  const other = await page.context().newPage();
  const errors: string[] = [];
  other.on("pageerror", (e) => errors.push(e.message));
  await other.goto("file://" + saved + "?theme=light");
  await expect(other.locator("svg.logix-diagram").first()).toBeVisible();
  expect(await other.locator("svg.logix-diagram").getAttribute("viewBox")).toBe(shown);
  expect(errors).toEqual([]);
});
