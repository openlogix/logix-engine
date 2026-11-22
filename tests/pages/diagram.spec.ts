// The diagram page opens, draws the design and offers Export.
import { test, expect } from "@playwright/test";
import { PAGES } from "./helpers";
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
