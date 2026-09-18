import React from "react";
import { createRoot } from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { Toaster } from "sonner";
import { commands } from "../../src/bindings";
import { ScreenshotLightbox } from "../../src/components/settings/history/ScreenshotLightbox";
import translations from "../../src/i18n/locales/en/translation.json";
import "../../src/App.css";

await i18n
  .use(initReactI18next)
  .init({ lng: "en", resources: { en: { translation: translations } } });
const image = (width: number, height: number, text: string) => {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d")!;
  context.fillStyle = "#17344e";
  context.fillRect(0, 0, width, height);
  context.fillStyle = "#ffffff";
  context.font = "64px sans-serif";
  context.fillText(text, 80, 160);
  context.strokeStyle = "#cbd5e1";
  for (let x = 0; x < width; x += 200) {
    context.beginPath();
    context.moveTo(x, 0);
    context.lineTo(x, height);
    context.stroke();
  }
  return canvas.toDataURL("image/png");
};
const images = [
  image(3200, 1200, "Panorama"),
  image(1600, 1200, "Screen 1"),
  image(1600, 1200, "Screen 2"),
];
commands.getHistoryScreenshot = async () => ({
  status: "ok",
  data: location.search.includes("legacy") ? images.slice(0, 1) : images,
});
commands.copyHistoryScreenshot = async (id, index) => {
  document.body.dataset.copied = `${id}:${index}`;
  return { status: "ok", data: null };
};
createRoot(document.getElementById("root")!).render(
  <>
    <Toaster />
    <ScreenshotLightbox
      entryId={42}
      onClose={() => {
        document.body.dataset.closed = "true";
      }}
    />
  </>,
);
