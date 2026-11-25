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
  expect(its.some((t) => /each bit/.test(t))).toBe(false);
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

// The formats, and Signed under them (R59; the user's design, 2026-11-25).
const format = (page: Page, name: string) => page.locator(".ctx [data-format]", { hasText: new RegExp(`^${name}$`) });
const signedBox = (page: Page) => page.getByRole("menuitemcheckbox", { name: "Signed" });
const height = (page: Page, name: string) => row(page, name).evaluate((n) => n.getBoundingClientRect().height);

test("a bus's menu has the formats and Signed, and no checkboxes for its bits or a line", async ({ page }) => {
  await withBits(page);
  const its = await menuOf(page, row(page, "pattern"));
  expect(its.some((t) => /each bit|as analog/.test(t))).toBe(false);
  expect(await page.locator(".ctx [data-format]").allTextContents()).toEqual(["Binary", "Hex", "ASCII", "Decimal", "Analog"]);
  await expect(signedBox(page)).toHaveCount(1);
});

test("an analog bus stays analog while its bits show, always two rows tall", async ({ page }) => {
  await withBits(page);
  await row(page, "pattern").locator(".fold.bits").click(); // hide the bits
  const flat = await height(page, "pattern");
  await menuOf(page, row(page, "pattern"));
  await format(page, "Analog").click();
  expect(await height(page, "pattern")).toBeCloseTo(flat * 2, 0);
  await row(page, "pattern").locator(".fold.bits").click(); // the bits again
  expect(await afterBus(page)).toEqual(BITS);
  expect(await height(page, "pattern")).toBeCloseTo(flat * 2, 0);
  await menuOf(page, row(page, "pattern"));
  await expect(format(page, "Analog")).toHaveClass(/on/);
  await page.keyboard.press("Escape");
  await row(page, "pattern").locator(".fold.bits").click(); // and hidden: still analog, the same height
  expect(await height(page, "pattern")).toBeCloseTo(flat * 2, 0);
});

test("Signed applies to Decimal and Analog, and keeps its state under the other formats", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await format(page, "Decimal").click();
  await menuOf(page, row(page, "pattern"));
  await expect(signedBox(page)).toBeEnabled();
  await signedBox(page).click();
  await menuOf(page, row(page, "pattern"));
  await expect(signedBox(page)).toHaveAttribute("aria-checked", "true");
  await format(page, "Hex").click();
  await menuOf(page, row(page, "pattern"));
  await expect(signedBox(page)).toBeDisabled();
  await expect(signedBox(page)).toHaveAttribute("aria-checked", "true");
  await format(page, "Analog").click();
  await menuOf(page, row(page, "pattern"));
  await expect(signedBox(page)).toBeEnabled();
  await expect(signedBox(page)).toHaveAttribute("aria-checked", "true");
});

test("saved views keep the fields older viewers read", async ({ page }) => {
  await withBits(page);
  await menuOf(page, row(page, "pattern"));
  await format(page, "Analog").click();
  await menuOf(page, row(page, "pattern"));
  await signedBox(page).click();
  const saved = async () => {
    const writes = (await hostCalls(page)).filter((c) => c[0] === "writeDeclared");
    const v = writes.length ? JSON.parse(writes[writes.length - 1][2] as string) : null;
    return v && v.rows.find((r: { signal?: string }) => r.signal?.endsWith(".pattern"));
  };
  await expect.poll(saved).toMatchObject({ show_as: "signed", analog: true, expanded: true });
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

// Adaptive scale (R59; the user's request, 2026-11-25): at the run's start
// the Morse beacon is on its first character, so the range of char_index
// shown there is smaller than the whole run's.
test("Adaptive scale fits an analog line's range to the shown time range", async ({ page }) => {
  await open(page, "plain");
  await showSignal(page, "dut", "char_index");
  // The scale in use, as the name's hover box gives it: "Analog scale: 0 to 99 (whole run)".
  const range = async () => {
    const m = /Analog scale: (\S+) to (\S+) \((.+)\)/.exec((await row(page, "char_index").getAttribute("title")) || "");
    return m ? { lo: +m[1], hi: +m[2], which: m[3] } : null;
  };
  await menuOf(page, row(page, "char_index"));
  await expect(page.getByRole("menuitemcheckbox", { name: "Adaptive scale" })).toBeDisabled();
  await format(page, "Analog").click();
  const whole = (await range())!;
  expect(whole.which).toBe("whole run");
  // Zoom in at the run's start, where the counter hasn't counted yet.
  const w = (await page.locator("#wavecol").boundingBox())!;
  await page.mouse.move(w.x + w.width * 0.05, w.y + 40);
  for (let k = 0; k < 3; k++) await page.mouse.wheel(0, -400);
  await page.waitForTimeout(400);
  expect(await range()).toEqual(whole); // off: the whole run's range
  await menuOf(page, row(page, "char_index"));
  await page.getByRole("menuitemcheckbox", { name: "Adaptive scale" }).click();
  // On: the range of the values shown, smaller than the whole run's.
  await expect.poll(async () => (await range())!.which).toBe("shown range");
  const shown = (await range())!;
  expect(shown.hi).toBeLessThan(whole.hi);
  expect([shown.lo, shown.hi]).not.toEqual([whole.lo, whole.hi]);
  const saved = async () => {
    const writes = (await hostCalls(page)).filter((c) => c[0] === "writeDeclared");
    const v = writes.length ? JSON.parse(writes[writes.length - 1][2] as string) : null;
    return v && v.rows.find((r: { signal?: string }) => r.signal?.endsWith(".char_index"));
  };
  await expect.poll(saved).toMatchObject({ analog: true, adaptive: true });
});
