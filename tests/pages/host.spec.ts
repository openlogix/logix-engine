// The page contract (docs/tool-api.md, Logix Viewer): both pages ask their
// host through window.logixHost, the one way for a host's own window (the
// user's decision, 2026-11-22).
import { test, expect } from "@playwright/test";
import { open, row, hostCalls, hostScript, PAGES } from "./helpers";
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
  expect(errors).toEqual([]);
});
