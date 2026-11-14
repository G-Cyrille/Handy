import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  testMatch: ["meeting-workspace.spec.ts", "screenshot-viewer.spec.ts"],
  workers: 1,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:1442",
    viewport: { width: 1200, height: 850 },
  },
  webServer: {
    command: "bunx vite --host 127.0.0.1 --port 1442 --strictPort",
    url: "http://127.0.0.1:1442",
    reuseExistingServer: false,
  },
});
