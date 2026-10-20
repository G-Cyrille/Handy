import React, { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Maximize, Minus, Plus } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";

type ScreenshotState =
  | { status: "loading" }
  | { status: "ready"; images: string[] }
  | { status: "error" };

/** Inspect the panorama or one monitor, preserving image pixels when copying. */
export const ScreenshotLightbox: React.FC<{
  entryId: number;
  onClose: () => void;
}> = ({ entryId, onClose }) => {
  const { t } = useTranslation();
  const [state, setState] = useState<ScreenshotState>({ status: "loading" });
  const [selected, setSelected] = useState(0);
  const [zoom, setZoom] = useState(1);
  const [fit, setFit] = useState(true);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [copying, setCopying] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    x: number;
    y: number;
    left: number;
    top: number;
  } | null>(null);
  const key = "settings.history.screenshot";
  const fitScale =
    size.width && viewport.width
      ? Math.min(viewport.width / size.width, viewport.height / size.height, 1)
      : 1;
  const scale = fit ? fitScale : zoom;

  useEffect(() => {
    let cancelled = false;
    setState({ status: "loading" });
    setSelected(0);
    commands
      .getHistoryScreenshot(entryId)
      .then((result) => {
        if (!cancelled)
          setState(
            result.status === "ok" && result.data.length
              ? { status: "ready", images: result.data }
              : { status: "error" },
          );
      })
      .catch(() => {
        if (!cancelled) setState({ status: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, [entryId]);

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const observer = new ResizeObserver(() =>
      setViewport({
        width: element.clientWidth,
        height: element.clientHeight,
      }),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [state.status]);

  const changeZoom = useCallback(
    (next: number, point?: { x: number; y: number }) => {
      const element = container.current;
      const nextScale = Math.max(Math.min(fitScale, 0.25), Math.min(8, next));
      if (element) {
        const x = point?.x ?? element.clientWidth / 2;
        const y = point?.y ?? element.clientHeight / 2;
        const imageX =
          (element.scrollLeft +
            x -
            Math.max(0, (element.clientWidth - size.width * scale) / 2)) /
          scale;
        const imageY =
          (element.scrollTop +
            y -
            Math.max(0, (element.clientHeight - size.height * scale) / 2)) /
          scale;
        requestAnimationFrame(() => {
          element.scrollLeft =
            imageX * nextScale +
            Math.max(0, (element.clientWidth - size.width * nextScale) / 2) -
            x;
          element.scrollTop =
            imageY * nextScale +
            Math.max(0, (element.clientHeight - size.height * nextScale) / 2) -
            y;
        });
      }
      setZoom(nextScale);
      setFit(false);
    },
    [fitScale, scale, size],
  );

  useEffect(() => {
    const element = container.current;
    if (!element) return;
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey && !event.metaKey) return;
      event.preventDefault();
      const bounds = element.getBoundingClientRect();
      changeZoom(scale * Math.exp(-event.deltaY * 0.01), {
        x: event.clientX - bounds.left,
        y: event.clientY - bounds.top,
      });
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [changeZoom, scale, state.status]);

  const copy = async () => {
    setCopying(true);
    try {
      const result = await commands.copyHistoryScreenshot(entryId, selected);
      if (result.status === "error") throw new Error(result.error);
      toast.success(t(`${key}.copied`));
    } catch (error) {
      console.error("Failed to copy screenshot:", error);
      toast.error(t(`${key}.copyFailed`));
    } finally {
      setCopying(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={t(`${key}.title`)}
      closeLabel={t(`${key}.close`)}
      className="!max-w-none h-[calc(100dvh-3rem)]"
      contentClassName="flex min-h-0 flex-1 flex-col !p-0 !overflow-hidden"
      contentFades={false}
    >
      {state.status === "ready" ? (
        <>
          <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-mid-gray/20 px-3 py-2">
            {state.images.length > 1 && (
              <select
                aria-label={t(`${key}.selectImage`)}
                className="bg-background text-text border border-mid-gray/30 rounded px-2 py-1 text-sm"
                value={selected}
                onChange={(event) => {
                  setSelected(Number(event.target.value));
                  setFit(true);
                  setSize({ width: 0, height: 0 });
                  container.current?.scrollTo(0, 0);
                }}
              >
                {state.images.map((_, index) => (
                  <option key={index} value={index}>
                    {index === 0
                      ? t(`${key}.panorama`)
                      : t(`${key}.monitor`, { number: index })}
                  </option>
                ))}
              </select>
            )}
            <Button
              size="sm"
              variant="ghost"
              aria-label={t(`${key}.zoomOut`)}
              onClick={() => changeZoom(scale / 1.25)}
            >
              <Minus size={16} />
            </Button>
            <output
              className="min-w-12 text-center text-sm tabular-nums"
              aria-label={t(`${key}.zoomLevel`)}
            >
              {Math.round(scale * 100)}%
            </output>
            <Button
              size="sm"
              variant="ghost"
              aria-label={t(`${key}.zoomIn`)}
              onClick={() => changeZoom(scale * 1.25)}
            >
              <Plus size={16} />
            </Button>
            <Button size="sm" variant="ghost" onClick={() => changeZoom(1)}>
              {t(`${key}.actualSize`)}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="flex items-center gap-1"
              onClick={() => setFit(true)}
            >
              <Maximize size={14} />
              {t(`${key}.fit`)}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              className="ml-auto flex items-center gap-2"
              disabled={copying}
              onClick={copy}
            >
              <Copy size={14} />
              {t(`${key}.copy`)}
            </Button>
          </div>
          <div
            ref={container}
            className="min-h-0 flex-1 overflow-auto bg-black/10 cursor-grab active:cursor-grabbing touch-none"
            tabIndex={0}
            role="region"
            aria-label={t(`${key}.navigation`)}
            onKeyDown={(event) => {
              if (event.key === "+" || event.key === "=") {
                event.preventDefault();
                changeZoom(scale * 1.25);
              }
              if (event.key === "-") {
                event.preventDefault();
                changeZoom(scale / 1.25);
              }
              if (event.key === "0") {
                event.preventDefault();
                setFit(true);
              }
            }}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.currentTarget.focus();
              event.currentTarget.setPointerCapture(event.pointerId);
              drag.current = {
                x: event.clientX,
                y: event.clientY,
                left: event.currentTarget.scrollLeft,
                top: event.currentTarget.scrollTop,
              };
            }}
            onPointerMove={(event) => {
              if (!drag.current) return;
              event.currentTarget.scrollLeft =
                drag.current.left + drag.current.x - event.clientX;
              event.currentTarget.scrollTop =
                drag.current.top + drag.current.y - event.clientY;
            }}
            onPointerUp={(event) => {
              drag.current = null;
              event.currentTarget.releasePointerCapture(event.pointerId);
            }}
            onPointerCancel={() => {
              drag.current = null;
            }}
            onLostPointerCapture={() => {
              drag.current = null;
            }}
          >
            <div
              className="grid min-h-full min-w-full place-items-center"
              style={{ width: size.width * scale, height: size.height * scale }}
            >
              <img
                key={selected}
                src={state.images[selected]}
                alt={t(`${key}.alt`)}
                draggable={false}
                onLoad={(event) =>
                  setSize({
                    width: event.currentTarget.naturalWidth,
                    height: event.currentTarget.naturalHeight,
                  })
                }
                className="block max-w-none select-none"
                style={
                  size.width
                    ? { width: size.width * scale, height: size.height * scale }
                    : { maxWidth: "100%" }
                }
              />
            </div>
          </div>
        </>
      ) : (
        <p className="py-8 text-center text-sm text-text/60">
          {t(
            `${key}.${state.status === "loading" ? "loading" : "unavailable"}`,
          )}
        </p>
      )}
    </Dialog>
  );
};
