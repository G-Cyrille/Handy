import React from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play } from "lucide-react";
import type { PendingTranscript, Segment, Session } from "@/bindings";
import { formatTimeOfDay, segmentStartMs, trackLabel } from "./meetingFormat";
import type { SegmentPlayer } from "./useSegmentPlayer";

interface TranscriptLinesProps {
  session: Session;
  segments: Segment[];
  /** Live, not yet finalized text for the segment being recognized. */
  pending?: PendingTranscript | null;
  player: SegmentPlayer;
}

const LINE_GRID =
  "grid grid-cols-[1rem_4.75rem_3.25rem_minmax(0,1fr)] gap-2 px-4 py-1 text-sm";

/** Chronological transcript: time of day, track, then text or its state. */
export const TranscriptLines: React.FC<TranscriptLinesProps> = ({
  session,
  segments,
  pending,
  player,
}) => {
  const { t, i18n } = useTranslation();
  const time = (startSample: number) =>
    formatTimeOfDay(
      segmentStartMs(session, { start_sample: startSample }),
      i18n.language,
    );

  return (
    <div className="py-1">
      {segments.map((segment) => {
        // The segment being recognized is shown by the pending line below.
        if (segment.id === pending?.segment_id) return null;
        const playing = player.playingId === segment.id;
        return (
          <div key={segment.id} className={LINE_GRID}>
            <button
              type="button"
              onClick={() => player.toggle(segment)}
              className="pt-0.5 text-text/50 hover:text-logo-primary cursor-pointer"
              title={
                playing
                  ? t("meetings.transcript.pause")
                  : t("meetings.transcript.play")
              }
              aria-label={
                playing
                  ? t("meetings.transcript.pause")
                  : t("meetings.transcript.play")
              }
            >
              {playing ? (
                <Pause width={14} height={14} fill="currentColor" />
              ) : (
                <Play width={14} height={14} fill="currentColor" />
              )}
            </button>
            <span className="text-xs text-text/50 tabular-nums whitespace-nowrap pt-0.5">
              {time(segment.start_sample)}
            </span>
            <span className="text-xs text-text/70 pt-0.5 truncate">
              {trackLabel(t, segment.track)}
            </span>
            {segment.asr_error !== null ? (
              <span className="text-red-500">
                {t("meetings.transcript.failed", { error: segment.asr_error })}
              </span>
            ) : segment.text === null ? (
              <span className="text-text/50 italic">
                {t("meetings.transcript.queued")}
              </span>
            ) : (
              <span className="select-text cursor-text">
                {segment.asr_status === "no_speech"
                  ? t("meetings.transcript.noSpeech")
                  : segment.text}
              </span>
            )}
          </div>
        );
      })}
      {pending && (
        <div className={LINE_GRID} title={t("meetings.transcript.provisional")}>
          <span />
          <span className="text-xs text-text/50 tabular-nums whitespace-nowrap pt-0.5">
            {time(pending.start_sample)}
          </span>
          <span className="text-xs text-text/70 pt-0.5 truncate">
            {trackLabel(t, pending.track)}
          </span>
          <span className="text-text/70">
            {pending.committed}
            {pending.tentative && (
              <span className="text-text/40 italic"> {pending.tentative}</span>
            )}
          </span>
        </div>
      )}
      {player.error && (
        <p className="px-4 py-1 text-sm text-red-500">
          {t("meetings.transcript.playbackError", { error: player.error })}
        </p>
      )}
    </div>
  );
};
