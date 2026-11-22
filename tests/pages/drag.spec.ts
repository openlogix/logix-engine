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
