import { test, expect } from "@playwright/test";

test("zoom, pan, fit and copy each monitor", async ({ page }) => {
  await page.goto("/tests/fixtures/screenshot-viewer.html");
  const region = page.getByRole("region", { name: /^Image:/ });
  const image = page.getByRole("img");
  await expect(image).toBeVisible();
  await expect
    .poll(() =>
      image.evaluate((element) => (element as HTMLImageElement).width),
    )
    .toBeLessThan(3200);
  await page.screenshot({ path: test.info().outputPath("viewer.png") });
  await page.getByRole("button", { name: "100%", exact: true }).click();
  await expect
    .poll(() =>
      image.evaluate((element) => (element as HTMLImageElement).width),
    )
    .toBe(3200);
  const before = await region.evaluate((element) => element.scrollLeft);
  const box = (await region.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width / 2 - 120,
    box.y + box.height / 2 - 80,
  );
  await page.mouse.up();
  await expect
    .poll(() => region.evaluate((element) => element.scrollLeft))
    .toBeGreaterThan(before);
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await expect(page.getByLabel("Zoom", { exact: true })).toHaveText("125%");
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  await expect
    .poll(() =>
      region.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBeLessThanOrEqual(1);
  for (const index of [0, 1, 2]) {
    await page.getByRole("combobox").selectOption(String(index));
    await page.getByRole("button", { name: "Copy image", exact: true }).click();
    await expect(page.locator("body")).toHaveAttribute(
      "data-copied",
      `42:${index}`,
    );
  }
  await page.keyboard.press("Escape");
  await expect(page.locator("body")).toHaveAttribute("data-closed", "true");
});

test("legacy panorama remains viewable and copyable", async ({ page }) => {
  await page.goto("/tests/fixtures/screenshot-viewer.html?legacy");
  await expect(page.getByRole("img")).toBeVisible();
  await expect(page.getByRole("combobox")).toHaveCount(0);
  await page.getByRole("button", { name: "Copy image", exact: true }).click();
  await expect(page.locator("body")).toHaveAttribute("data-copied", "42:0");
});
