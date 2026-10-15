import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type MeetingSnapshot,
  type MeetingTrackStatus,
  type Session,
} from "@/bindings";
import { Button } from "../ui/Button";
import {
  SEGMENT_PAGE_SIZE,
  TRACK,
  errorMessage,
  formatClock,
  samplesToMs,
  trackLabel,
  unwrap,
} from "./meetingFormat";
import { Switch } from "./Switch";
import { TranscriptLines } from "./TranscriptLines";
import { usePolling } from "./usePolling";
import { useSegmentPlayer } from "./useSegmentPlayer";

/** A track that received nothing for this long is reported as stalled. */
const STALL_MS = 3_000;
/** Bounds how many pages one poll may fetch when catching up. */
const OPS_ROW =
  "grid grid-cols-[6.5rem_minmax(0,1fr)_auto] items-center gap-3 px-4 py-2 text-sm";

/** Fetch a bounded tail; older transcript remains paginated on disk. */
const useLiveDetail = (sessionId: string) =>
  usePolling(async () => {
    const first = await unwrap(commands.getMeetingDetail(sessionId, 0, 1));
    const offset = Math.max(0, first.total_segments - SEGMENT_PAGE_SIZE);
    const detail = await unwrap(
      commands.getMeetingDetail(sessionId, offset, SEGMENT_PAGE_SIZE),
    );
    return {
      segments: detail.segments,
      total: detail.total_segments,
      insights: detail.insights,
    };
  }, 1000);

type TrackState =
  | { kind: "waiting" }
  | { kind: "receiving" }
  | { kind: "stalled"; silentMs: number };

const trackState = (
  status: MeetingTrackStatus | undefined,
  elapsedMs: number,
): TrackState => {
  if (!status || status.last_received_ms === 0) {
    return elapsedMs > STALL_MS
      ? { kind: "stalled", silentMs: elapsedMs }
      : { kind: "waiting" };
  }
  const silentMs = elapsedMs - status.last_received_ms;
  return silentMs > STALL_MS
    ? { kind: "stalled", silentMs }
    : { kind: "receiving" };
};

interface LiveMeetingProps {
  snapshot: MeetingSnapshot;
  session: Session;
  onStopped: (sessionId: string) => void;
}

/** Operations table for the running capture, with the live transcript below. */
export const LiveMeeting: React.FC<LiveMeetingProps> = ({
  snapshot,
  session,
  onStopped,
}) => {
  const { t } = useTranslation();
  const live = useLiveDetail(session.id);
  const player = useSegmentPlayer();
  const [stopping, setStopping] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const [aiPending, setAiPending] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const followRef = useRef(true);
  const endRef = useRef<HTMLDivElement>(null);

  // Keep following new lines only while the end of the transcript is visible.
  useEffect(() => {
    const end = endRef.current;
    if (!end) return;
    const observer = new IntersectionObserver(([entry]) => {
      followRef.current = entry.isIntersecting;
    });
    observer.observe(end);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (followRef.current) endRef.current?.scrollIntoView({ block: "end" });
  }, [live.data, snapshot.pending]);

  const stop = async () => {
    setStopping(true);
    setStopError(null);
    try {
      await unwrap(commands.stopMeeting());
      onStopped(session.id);
    } catch (e) {
      setStopError(errorMessage(e));
      setStopping(false);
    }
  };

  const setAi = async (enabled: boolean) => {
    setAiPending(true);
    setAiError(null);
    try {
      await unwrap(commands.setMeetingAi(enabled));
    } catch (e) {
      setAiError(errorMessage(e));
    } finally {
      setAiPending(false);
    }
  };

  const insights = live.data?.insights;
  const aiConfigured = snapshot.provider !== "";
  const pending =
    snapshot.pending?.session_id === session.id ? snapshot.pending : null;

  const trackRow = (track: number, source: string) => {
    const status = snapshot.tracks.find((item) => item.track === track);
    const state = trackState(status, snapshot.elapsed_ms);
    const recorded = formatClock(samplesToMs(status?.received_samples ?? 0));
    return (
      <div key={track} className={OPS_ROW}>
        <span className="font-medium">{trackLabel(t, track)}</span>
        <span className="truncate text-text/70" title={source}>
          {source}
        </span>
        <span
          className={`text-end tabular-nums ${state.kind === "stalled" ? "text-red-500" : ""}`}
        >
          {state.kind === "stalled"
            ? t("meetings.live.stalled", {
                duration: formatClock(state.silentMs),
              })
            : state.kind === "waiting"
              ? t("meetings.live.waiting")
              : recorded}
        </span>
      </div>
    );
  };

  return (
    <div className="space-y-2">
      <div className="sticky top-0 z-10 bg-background border border-mid-gray/20 rounded-lg divide-y divide-mid-gray/20">
        <div className="flex items-center gap-3 px-4 py-2">
          <span className="w-2 h-2 rounded-full bg-red-500 shrink-0" />
          <span className="text-sm font-medium">
            {t("meetings.live.recording")}
          </span>
          <span className="text-lg font-semibold tabular-nums">
            {formatClock(snapshot.elapsed_ms)}
          </span>
          <span className="flex-1" />
          <Button onClick={stop} disabled={stopping}>
            {stopping ? t("meetings.live.stopping") : t("meetings.live.stop")}
          </Button>
        </div>
        {stopError && (
          <p className="px-4 py-2 text-sm text-red-500">
            {t("meetings.live.stopError", { error: stopError })}
          </p>
        )}
        {snapshot.capture_error && (
          <p className="px-4 py-2 text-sm text-red-500">
            {snapshot.capture_error}
          </p>
        )}
        {trackRow(TRACK.microphone, session.microphone)}
        {trackRow(TRACK.system, session.system)}
        <div className={OPS_ROW}>
          <span className="font-medium">
            {t("meetings.live.transcription")}
          </span>
          <span />
          <span className="text-end tabular-nums">
            {snapshot.backlog_ms > 0
              ? t("meetings.live.behind", {
                  duration: formatClock(snapshot.backlog_ms),
                })
              : t("meetings.live.upToDate")}
          </span>
        </div>
        <div className={OPS_ROW}>
          <span className="font-medium">{t("meetings.ai.title")}</span>
          <span className="truncate text-text/70">
            {aiConfigured
              ? `${snapshot.provider} · ${snapshot.model}`
              : t("meetings.ai.unconfigured")}
          </span>
          <span className="flex items-center justify-end gap-3">
            <span
              className={`tabular-nums ${insights?.error ? "text-red-500" : ""}`}
            >
              {!session.ai_enabled
                ? t("meetings.ai.off")
                : (insights?.error ??
                  t("meetings.ai.count", {
                    count: insights?.suggestions.length ?? 0,
                  }))}
            </span>
            <Switch
              checked={session.ai_enabled}
              onChange={setAi}
              disabled={aiPending || (!aiConfigured && !session.ai_enabled)}
              label={t("meetings.ai.title")}
            />
          </span>
        </div>
        {aiError && (
          <p className="px-4 py-2 text-sm text-red-500">
            {t("meetings.ai.toggleError", { error: aiError })}
          </p>
        )}
      </div>

      {session.ai_enabled && insights && insights.suggestions.length > 0 && (
        <div className="border border-mid-gray/20 rounded-lg py-1">
          {insights.suggestions.map((suggestion, index) => (
            <div
              key={`${suggestion.at_ms}-${index}`}
              className="grid grid-cols-[4.5rem_minmax(0,1fr)] gap-2 px-4 py-1 text-sm"
            >
              <span className="text-xs text-text/50 tabular-nums pt-0.5">
                {formatClock(suggestion.at_ms)}
              </span>
              <span className="text-logo-primary select-text cursor-text">
                {suggestion.text}
              </span>
            </div>
          ))}
        </div>
      )}

      <div>
        {live.error && (
          <p className="px-4 py-1 text-sm text-red-500">
            {t("meetings.live.transcriptError", { error: live.error })}
          </p>
        )}
        {live.data && live.data.total > live.data.segments.length && (
          <p className="px-4 py-1 text-sm text-text/60">
            {t("meetings.live.truncated", { count: SEGMENT_PAGE_SIZE })}
          </p>
        )}
        {live.data?.total === 0 && !pending ? (
          <p className="px-4 py-3 text-sm text-text/60">
            {t("meetings.live.empty")}
          </p>
        ) : (
          <TranscriptLines
            session={session}
            segments={live.data?.segments ?? []}
            pending={pending}
            player={player}
          />
        )}
        <div ref={endRef} className="h-1" />
      </div>
    </div>
  );
};
