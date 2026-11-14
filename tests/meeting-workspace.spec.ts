import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: { os_type: "macos", platform: "macos" },
      __TAURI_INTERNALS__: {
        metadata: { currentWindow: { label: "meeting-42" } },
        invoke: async (command: string, args?: { path?: string }) => {
          if (command === "plugin:opener|open_path")
            document.body.dataset.openedPath = args?.path;
          return null;
        },
        convertFileSrc: () =>
          "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='400' height='180'%3E%3Crect width='400' height='180' fill='%23eee'/%3E%3C/svg%3E",
      },
    });
  });
  await page.goto("/tests/fixtures/meeting-workspace.html");
  await expect(
    page.getByRole("heading", { name: "Point mensuel" }),
  ).toBeVisible();
});

test("links in an agent's ongoing reply open through Handy and preserve the meeting", async ({
  page,
}) => {
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Créer le résultat");
  await page.getByRole("button", { name: "Envoyer le message" }).click();
  await page.getByRole("link", { name: "Résultat en cours" }).click();
  await expect(page.locator("body")).toHaveAttribute(
    "data-opened-path",
    "/fixture/result.txt",
  );
  await expect(
    page.getByRole("heading", { name: "Point mensuel" }),
  ).toBeVisible();
  await expect(page).toHaveURL(/meeting-workspace\.html$/);
});

test("one chat retains its draft across panes, while actions launch independent agents", async ({
  page,
}) => {
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Garder les sources");
  await page.getByRole("button", { name: "Préparation", exact: true }).click();
  await expect(input).toHaveValue("Garder les sources");
  await expect(
    page.getByLabel("Actions proposées", { exact: true }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "En direct", exact: true }).click();
  await expect(input).toHaveValue("Garder les sources");
  await page
    .getByRole("button", { name: "Rapport mensuel + Share", exact: true })
    .click();
  await expect(
    page.getByRole("button", {
      name: "Arrêter l’agent Rapport mensuel + Share",
    }),
  ).toBeVisible();
  await input.fill("Ajoute les sources");
  await page.getByRole("button", { name: "Envoyer le message" }).click();
  await expect(page.getByRole("log")).toContainText("Ajoute les sources");
  await page
    .getByRole("button", { name: "Arrêter l’agent Rapport mensuel + Share" })
    .click();
  await expect(page.getByRole("log")).toContainText("Ajoute les sources");
  await expect(
    page.getByRole("button", { name: "Arrêter", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Principal", exact: true }).click();
  await expect(input).toHaveValue("Garder les sources");
});

test("timeline supports notes, capture scope, simple selection and compact chat", async ({
  page,
}) => {
  await page
    .getByRole("checkbox", { name: "Sélectionner le passage à 0:05" })
    .check();
  await page.getByRole("button", { name: "+ Note", exact: true }).click();
  await page.getByLabel("Ma note · 0:05").fill("À vérifier avant vendredi");
  await page.getByRole("button", { name: "Insérer la note" }).click();
  await expect(page.locator(".meeting-entry-note")).toContainText(
    "À vérifier avant vendredi",
  );
  await page.getByRole("button", { name: "Tout désélectionner" }).click();
  await expect(
    page.getByRole("checkbox", { name: "Sélectionner le passage à 0:05" }),
  ).not.toBeChecked();
  await expect(page.getByLabel("Portée de la capture")).toHaveValue("all");
  await page.getByLabel("Portée de la capture").selectOption("region");
  await page.getByRole("button", { name: "Capture", exact: true }).click();
  await expect(page.locator(".meeting-entry-capture img")).toBeVisible();
  await page.getByRole("button", { name: "Mode compact", exact: true }).click();
  await page.setViewportSize({ width: 460, height: 740 });
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Message compact");
  await page.getByRole("button", { name: "Envoyer le message" }).click();
  await expect(page.getByRole("log")).toContainText("Message compact");
  await page.screenshot({
    path: test.info().outputPath("meeting-compact.png"),
  });
});

test("new discussions retain Principal and use the selected provider and model", async ({
  page,
}) => {
  const input = page.getByRole("textbox", { name: "Message", exact: true });
  await input.fill("Brouillon de Principal");
  await page
    .getByRole("button", { name: "Nouvelle discussion", exact: true })
    .click();
  await page.getByLabel("Nom de la discussion").fill("Analyse des documents");
  await page
    .getByRole("combobox", { name: "Assistant", exact: true })
    .selectOption("claude");
  await page.getByLabel("Modèle", { exact: true }).fill("modele-de-test");
  await input.fill("Analyser les documents joints");
  await page.getByRole("button", { name: "Envoyer le message" }).click();
  await expect(page.locator("body")).toHaveAttribute("data-provider", "claude");
  await expect(page.locator("body")).toHaveAttribute(
    "data-requested-model",
    "modele-de-test",
  );
  await page.getByRole("button", { name: "Principal", exact: true }).click();
  await expect(input).toHaveValue("Brouillon de Principal");
  await page
    .getByRole("button", {
      name: "Retirer la suggestion Rapport mensuel + Share",
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("button", { name: "Rapport mensuel + Share", exact: true }),
  ).toHaveCount(0);
});
