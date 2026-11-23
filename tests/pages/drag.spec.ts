// A row can be dragged without selecting it first (the user's report,
// 2026-11-22: starting a drag redrew the list, and WebKit cancelled it).
import { test, expect } from "@playwright/test";
import { open, rows } from "./helpers";

test("an unselected row can be dragged with the mouse", async ({ page }) => {
  await open(page, "plain");
  const before = await rows(page);
  await expect(page.locator("#names .nrow.sel")).toHaveCount(0);
  await page.locator("#names .nrow").nth(4).dragTo(page.locator("#names .nrow").nth(0), { targetPosition: { x: 20, y: 3 } });
  await expect.poll(() => rows(page)).toEqual([before[4], ...before.slice(0, 4), ...before.slice(5)]);
});

test("a row dropped above the list lands first, where its line showed", async ({ page }) => {
  // The user's report, 2026-11-23: dragged past the top, over the time
  // ruler, the line showed the first place and the row didn't move.
  await open(page, "plain");
  const before = await rows(page);
  const box = (await page.locator("#names").boundingBox())!;
  await page.locator("#names .nrow").nth(3).dragTo(page.locator("body"), { targetPosition: { x: box.x + 20, y: Math.max(2, box.y - 15) } });
  await expect.poll(() => rows(page)).toEqual([before[3], ...before.slice(0, 3), ...before.slice(4)]);
});
