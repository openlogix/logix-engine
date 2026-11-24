// Count cycles only for signals that look like clocks (R44), the clock
// picker's Other signals, and menus that act on a whole selection (R60);
// the user's rules, 2026-11-22.
import { test, expect } from "@playwright/test";
import { open, rows, row, menuOf, showSignal } from "./helpers";

test("Count cycles is offered for clock-like signals only", async ({ page }) => {
  await open(page, "plain");
  expect((await menuOf(page, row(page, "clk"))).some((t) => /Count cycles of clk/.test(t))).toBe(true);
  await page.keyboard.press("Escape");
  // led_on blinks Morse code: not a clock, in the rows or in the signal list.
  expect((await menuOf(page, row(page, "led_on"))).some((t) => /cycles/i.test(t))).toBe(false);
  await page.keyboard.press("Escape");
  expect((await menuOf(page, page.locator('.var[data-path="morse_tb.led_on"]'))).some((t) => /cycles/i.test(t))).toBe(false);
  await page.keyboard.press("Escape");
  // A counter's lowest bit toggles steadily; its top bit never does.
  await showSignal(page, "dut", "cycle_count");
  await row(page, "cycle_count").locator(".fold.bits").click();
  const steady = await menuOf(page, page.locator('.nrow.bit[title$="cycle_count[0]"]'));
  expect(steady.some((t) => /Count cycles of .*cycle_count\[0\]/.test(t))).toBe(true);
  await page.keyboard.press("Escape");
  expect((await menuOf(page, page.locator('.nrow.bit[title$="cycle_count[31]"]'))).some((t) => /cycles/i.test(t))).toBe(false);
});

test("the clock picker lists likely clocks, and Other signals the rest", async ({ page }) => {
  await open(page, "plain");
  await page.locator("#clockPick").click();
  const likely = await page.locator(".ctx .it").allTextContents();
  expect(likely.some((t) => /clk/.test(t))).toBe(true);
  expect(likely.some((t) => /led_on/.test(t))).toBe(false);
  await page.getByRole("menuitem", { name: /Other signals/ }).click();
  await expect(page.locator(".ctx .ttl")).toHaveText("Count the cycles of");
  await page.getByRole("menuitem", { name: /^led_on/ }).click();
  await expect(page.locator("#tbCycles")).toHaveClass(/on/);
  await expect(page.locator("#clockPick")).toContainText("led_on");
});

test("a right-click in a selection of several rows acts on all of them", async ({ page }) => {
  await open(page, "plain");
  await row(page, "clk").click();
  await row(page, "text").click({ modifiers: ["Shift"] });
  const its = await menuOf(page, row(page, "led_on"));
  await expect(page.locator(".ctx .ttl")).toHaveText("3 signals");
  expect(its.some((t) => /Hide these signals/.test(t))).toBe(true);
  expect(its.some((t) => /cycles|Copy|bit|analog|group/i.test(t))).toBe(false);
  await page.locator('.ctx [data-color="5"]').click();
  const colors = await page.locator("#names .nrow.sel").evaluateAll((ns) => ns.map((n) => (n as HTMLElement).style.getPropertyValue("--c")));
  expect(colors).toEqual(["var(--s5)", "var(--s5)", "var(--s5)"]);
  await menuOf(page, row(page, "led_on"));
  await page.getByRole("menuitem", { name: "Hide these signals" }).click();
  const r = await rows(page);
  for (const gone of ["clk", "led_on", "text"]) expect(r).not.toContain(gone);
  expect(r).toContain("text_length");
});

test("a right-click outside the selection selects that row alone", async ({ page }) => {
  await open(page, "plain");
  await row(page, "text_length").click();
  await row(page, "heard").click({ modifiers: ["Shift"] });
  await menuOf(page, row(page, "was_on"));
  await expect(page.locator(".ctx .ttl")).toContainText("was_on");
  await expect(page.locator("#names .nrow.sel")).toHaveCount(1);
});

test("a line parts the two group items from the two blank row items", async ({ page }) => {
  await open(page, "plain");
  await row(page, "clk").click({ button: "right" });
  // The menu's entries in order, a line as "-".
  const entries = await page.locator(".ctx > .it, .ctx > .sep").evaluateAll((els) =>
    els.map((e) => (e.classList.contains("sep") ? "-" : (e.textContent ?? "").trim())),
  );
  const at = entries.indexOf("Add a group below");
  expect(entries.slice(at - 1, at + 3)).toEqual([
    "Add a group above",
    "Add a group below",
    "-",
    "Add a blank row above",
  ]);
});
