import type { TFunction } from "i18next";
import type { Segment, Session } from "@/bindings";

/** Meeting tracks are stored as 16 kHz mono PCM. */
export const SAMPLE_RATE = 16_000;

/** Track index used by the backend for each capture source. */
export const TRACK = { microphone: 0, system: 1 } as const;

/** Maximum number of segments fetched or rendered at once. */
export const SEGMENT_PAGE_SIZE = 100;

type CommandResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: string };

/** Unwraps a tauri-specta result, throwing its error message. */
export const unwrap = async <T>(
  result: Promise<CommandResult<T>>,
): Promise<T> => {
  const resolved = await result;
  if (resolved.status === "error") throw new Error(resolved.error);
  return resolved.data;
};

export const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

export const samplesToMs = (samples: number): number =>
  (samples * 1000) / SAMPLE_RATE;

/** Wall-clock start of a segment, in epoch milliseconds. */
export const segmentStartMs = (
  session: Pick<Session, "started_at_ms">,
  segment: Pick<Segment, "start_sample">,
): number => session.started_at_ms + samplesToMs(segment.start_sample);

/** Formats a duration as `M:SS` or `H:MM:SS`. */
export const formatClock = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const seconds = String(total % 60).padStart(2, "0");
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
};

export const formatTimeOfDay = (epochMs: number, locale: string): string =>
  new Intl.DateTimeFormat(locale, {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  }).format(epochMs);

export const formatSessionDate = (epochMs: number, locale: string): string =>
  new Intl.DateTimeFormat(locale, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(epochMs);

export const trackLabel = (t: TFunction, track: number): string => {
  if (track === TRACK.microphone) return t("meetings.track.microphone");
  if (track === TRACK.system) return t("meetings.track.system");
  return t("meetings.track.unknown", { track });
};

export const sessionStatusLabel = (t: TFunction, status: string): string =>
  t(`meetings.status.${status}`, { defaultValue: status });

/** Text color for a session status; problems stand out. */
export const sessionStatusClass = (status: string): string => {
  if (status === "failed") return "text-red-500";
  if (status === "interrupted") return "text-warning";
  if (status === "recording") return "text-logo-primary";
  return "text-text/70";
};
