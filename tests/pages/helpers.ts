// Opening the pages the Rust tests wrote, and reading them as a person
// would: rows by name, menus by their items.
import { expect, type Page } from "@playwright/test";
import * as path from "node:path";
import * as fs from "node:fs";

/** The folder the Rust tests write the pages to. */
export const PAGES = path.resolve(__dirname, "../../target/tmp/pages");

/**
 * Opens a page (its name in PAGES, without .html) and collects its errors;
 * `saved` gets the views the page saves through its host.
 */
export async function open(page: Page, name: string) {
  const file = path.join(PAGES, name.endsWith(".html") ? name : `${name}.html`);
  if (!fs.existsSync(file)) throw new Error(`${file} is missing: run cargo test first`);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
  await page.addInitScript(() => {
    (window as unknown as { logixWaveSaveView: (v: unknown) => void }).logixWaveSaveView = (v) => {
      (window as unknown as { __saved: unknown }).__saved = v;
    };
  });
  await page.goto("file://" + file + "?theme=light");
  await expect(page.locator("#names")).toBeVisible();
  return { errors };
}

/** The rows of the Signal column, top to bottom: a signal by its last name, "G:<name>" a group, "blank". */
export function rows(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll("#names .nrow")].map((n) =>
      (n as HTMLElement).title
        ? (n as HTMLElement).title.split(".").pop()!
        : n.classList.contains("group")
          ? "G:" + (n.querySelector(".nm")?.textContent ?? "").trim()
          : "blank",
    ),
  );
}

/** A signal's row (not a bit row) by its name. */
export const row = (page: Page, name: string) =>
  page.locator("#names .nrow:not(.bit)").filter({ has: page.locator(".nm", { hasText: new RegExp(`^(dut\\.)?${name.replace(/[[\]]/g, "\\$&")}(\\[|$)`) }) }).first();

/** The open menu's items' texts. */
export const items = (page: Page) => page.locator(".ctx .it").allTextContents();

/** Right-clicks and returns the menu's items. */
export async function menuOf(page: Page, target: ReturnType<Page["locator"]>) {
  await target.click({ button: "right" });
  await expect(page.locator(".ctx")).toBeVisible();
  return items(page);
}

/** Shows a signal of the design from the signal list. */
export async function showSignal(page: Page, scope: string, name: string) {
  const header = page.locator(".scope-h", { hasText: scope }).first();
  if ((await header.getAttribute("aria-expanded")) !== "true") await header.click();
  await page.locator(`.var[data-path="morse_tb.${scope}.${name}"] .chk`).click();
}
