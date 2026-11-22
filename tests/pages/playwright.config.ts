// The engine's pages in WebKit, the engine behind Logix IDE's windows on
// macOS. The pages are written by the Rust tests (tests/sim.rs and
// tests/diagram.rs, "...the browser tests use") into target/tmp/pages, so
// run `cargo test` first: scripts/test.sh does both.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "*.spec.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: { viewport: { width: 1100, height: 640 } },
  projects: [{ name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 1100, height: 640 } } }],
});
