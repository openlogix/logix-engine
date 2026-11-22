// A bus's bits are part of its row (R58, the user's rules, 2026-11-22), and
// bits and analog turn each other off (R59).
import { test, expect, type Page } from "@playwright/test";
import { open, rows, row, menuOf, showSignal, hostCalls } from "./helpers";

const BITS = ["pattern[5]", "pattern[4]", "pattern[3]", "pattern[2]", "pattern[1]", "pattern[0]"];

/** The plain page, with dut.pattern shown and its bits expanded, and dut.char_index. */
async function withBits(page: Page) {
  const opened = await open(page, "plain");
  await showSignal(page, "dut", "pattern");
  await showSignal(page, "dut", "char_index");
  await row(page, "pattern").locator(".fold.bits").click();
  return opened;
}
const afterBus = async (page: Page) => {
  const r = await rows(page);
  const i = r.indexOf("pattern");
  return r.slice(i + 1, i + 7);
};

test("the bits sit right under their bus, in order, and can't be dragged", async ({ page }) => {
  const { errors } = await withBits(page);
  expect(await afterBus(page)).toEqual(BITS);
  await expect(page.locator("#names .nrow.bit").nth(2)).toHaveAttribute("draggable", "false");
  expect(errors).toEqual([]);
});

test("a bit's menu only reads it", async ({ page }) => {
  await withBits(page);
  const its = await menuOf(page, page.locator("#names .nrow.bit").nth(2));
  expect(its.some((t) => /group|blank|Hide this signal|analog/.test(t))).toBe(false);
  expect(its.some((t) => /Copy the name/.test(t))).toBe(true);
  await expect(page.getByRole("menuitemcheckbox", { name: /each bit/ })).toHaveAttribute("aria-checked", "true");
});

test("moving or deleting a bit alone does nothing", async ({ page }) => {
  await withBits(page);
  await page.locator("#names .nrow.bit").nth(2).click();
  const before = await rows(page);
  for (const key of ["Alt+ArrowDown", "Alt+ArrowUp", "Delete"]) await page.keyboard.press(key);
  expect(await rows(page)).toEqual(before);
});

test("a row added below the bus goes after its bits, and the bits move with the bus", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await page.getByRole("menuitem", { name: "Add a group below" }).click();
  await page.keyboard.press("Escape");
  let r = await rows(page);
  expect(r[r.indexOf("pattern") + 7]).toBe("G:New group");
  await row(page, "pattern").click();
  await page.keyboard.press("Alt+ArrowUp");
  await page.keyboard.press("Alt+ArrowUp");
  expect(await afterBus(page)).toEqual(BITS);
});

test("another row moved up past the bus lands above it, not between its bits", async ({ page }) => {
  await withBits(page);
  await row(page, "char_index").click();
  for (let k = 0; k < 4; k++) await page.keyboard.press("Alt+ArrowUp");
  expect(await afterBus(page)).toEqual(BITS);
  const r = await rows(page);
  expect(r.indexOf("char_index")).toBeLessThan(r.indexOf("pattern"));
});

test("a group dropped on a bit lands above the bus", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "char_index"));
  await page.getByRole("menuitem", { name: "Add a group below" }).click();
  await page.keyboard.press("Escape");
  // HTML drag and drop, as the page handles it.
  await page.evaluate(() => {
    const g = document.querySelector(".nrow.group")!, bit = document.querySelectorAll(".nrow.bit")[3], area = document.querySelector("#rows")!;
    const dt = new DataTransfer(), y = bit.getBoundingClientRect().top + 4;
    g.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
    area.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, clientY: y, dataTransfer: dt }));
    area.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, clientY: y, dataTransfer: dt }));
    g.dispatchEvent(new DragEvent("dragend", { bubbles: true, dataTransfer: dt }));
  });
  const r = await rows(page);
  expect(r[r.indexOf("pattern") - 1]).toBe("G:New group");
  expect(await afterBus(page)).toEqual(BITS);
});

test("the bits take their bus's color", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await page.locator('.ctx [data-color="4"]').click();
  await page.keyboard.press("Escape");
  const colors = await page.locator("#names .nrow.bit").evaluateAll((ns) => ns.map((n) => (n as HTMLElement).style.getPropertyValue("--c")));
  expect(colors).toHaveLength(6);
  expect(colors.every((c) => c.includes("--s4"))).toBe(true);
});

test("Show each bit and Show as analog are checkboxes that turn each other off", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await page.getByRole("menuitemcheckbox", { name: /analog/ }).click();
  expect((await rows(page)).some((n) => n.startsWith("pattern["))).toBe(false);
  await expect(row(page, "pattern").locator(".fold.bits")).toHaveCount(1);
  await menuOf(page, row(page, "pattern"));
  const eachBit = page.getByRole("menuitemcheckbox", { name: /each bit/ });
  await expect(eachBit).toHaveAttribute("aria-checked", "false");
  await expect(page.getByRole("menuitemcheckbox", { name: /analog/ })).toHaveAttribute("aria-checked", "true");
  await eachBit.click();
  expect(await afterBus(page)).toEqual(BITS);
  await menuOf(page, row(page, "pattern"));
  await expect(page.getByRole("menuitemcheckbox", { name: /analog/ })).toHaveAttribute("aria-checked", "false");
});

test("a selection into the bits takes all of them; a bit alone is just the bit", async ({ page }) => {
  await withBits(page);
  await row(page, "char_index").click();
  await page.locator("#names .nrow.bit").nth(1).click({ modifiers: ["Shift"] });
  await expect(page.locator("#names .nrow.bit.sel")).toHaveCount(6);
  await page.locator("#names .nrow.bit").nth(1).click();
  await expect(page.locator("#names .nrow.sel")).toHaveCount(1);
});

test("hiding the bus hides its bits, and undo brings both back", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await page.getByRole("menuitem", { name: "Hide this signal" }).click();
  expect((await rows(page)).some((n) => n.startsWith("pattern"))).toBe(false);
  // The page's Undo button: Ctrl+Z didn't reach the page in WebKit on
  // Linux CI (2026-11-22).
  await page.locator("#undoBtn").click();
  expect(await afterBus(page)).toEqual(BITS);
});

test("a saved view keeps the bus expanded and its bits by path", async ({ page }) => {
  await withBits(page);
  // The view goes to the host's writeDeclared("view", text), as the .view file's text.
  const lastView = async () => {
    const views = (await hostCalls(page)).filter((c) => c[0] === "writeDeclared" && c[1] === "view");
    return views.length ? JSON.parse(views[views.length - 1][2] as string) : null;
  };
  await expect.poll(async () => JSON.stringify(await lastView())).toContain("pattern[0]");
  const bus = (await lastView()).rows.find((r: { signal?: string }) => r.signal === "morse_tb.dut.pattern");
  expect(bus.expanded).toBe(true);
});
