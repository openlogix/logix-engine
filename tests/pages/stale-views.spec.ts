// A saved view that no longer matches the run (the testbench changed since
// it was saved): the page opens without errors and shows what's still there
// (the user's request, 2026-11-22).
import { test, expect } from "@playwright/test";
import { open, rows } from "./helpers";

test("signals that are gone are left out, the others stay in order", async ({ page }) => {
  const { errors } = await open(page, "stale-gone");
  expect(await rows(page)).toEqual(["clk", "G:Design", "led_on"]);
  expect(errors).toEqual([]);
});

test("a narrower bus shows the bits it has; a 1-bit signal isn't expanded or analog", async ({ page }) => {
  const { errors } = await open(page, "stale-width");
  const r = await rows(page);
  expect(r.slice(0, 7)).toEqual(["pattern", "pattern[5]", "pattern[4]", "pattern[3]", "pattern[2]", "pattern[1]", "pattern[0]"]);
  expect(r).not.toContain("pattern[9]");
  expect(r).toContain("led_on");
  await expect(page.locator("#names .nrow", { hasText: "led_on" }).locator(".fold.bits")).toHaveCount(0);
  // Analog is for buses and reals: a 1-bit row stays one row high.
  await expect(page.locator("#names .nrow", { hasText: "led_on" })).not.toHaveAttribute("style", /height/);
  expect(errors).toEqual([]);
});

test("a clock that's gone gives way to the likely one, with a note", async ({ page }) => {
  const { errors } = await open(page, "stale-clock");
  await expect(page.locator("#clockPick")).toContainText("clk");
  await expect(page.locator(".toast, #toast")).toContainText("morse_tb.old_clock");
  expect(errors).toEqual([]);
});

test("a zoom, cursor and marker past the run's end are brought inside it", async ({ page }) => {
  const { errors } = await open(page, "stale-range");
  // The run is 71 µs: the zoom shows it, and the cursor (950 µs) and the
  // marker (960 µs) are dropped.
  const status = page.locator("#status");
  await expect(status).toContainText("Showing 0 µs – 71.042 µs");
  await expect(status).toContainText("Cursor –");
  await expect(status).not.toContainText("Marker");
  expect(errors).toEqual([]);
});

test("a view with nothing left opens the default view", async ({ page }) => {
  const { errors } = await open(page, "stale-empty");
  const r = await rows(page);
  expect(r.length).toBeGreaterThan(0);
  expect(r[0]).toBe("clk");
  expect(errors).toEqual([]);
});

test("rows the viewer never wrote are skipped, and unknown formats and colors become the defaults", async ({ page }) => {
  const { errors } = await open(page, "stale-garbage");
  expect(await rows(page)).toEqual(["led_on"]);
  const led = page.locator("#names .nrow", { hasText: "led_on" });
  await expect(led).toHaveAttribute("style", /--c:var\(--s1\)/);
  expect(errors).toEqual([]);
});
