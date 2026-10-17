import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, FolderOpen } from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  commands,
  type MeetingDetail,
  type TranscriptFormat,
} from "@/bindings";
import { Button } from "../ui/Button";
import {
  SEGMENT_PAGE_SIZE,
  errorMessage,
  formatClock,
  formatSessionDate,
  sessionStatusClass,
  sessionStatusLabel,
  unwrap,
} from "./meetingFormat";
import { TranscriptLines } from "./TranscriptLines";
import { useSegmentPlayer } from "./useSegmentPlayer";

/** While a page still has segments awaiting ASR, refresh it this often. */
const PENDING_REFRESH_MS = 2_000;
const TRANSCRIPT_FORMATS = [
  "markdown",
  "text",
  "json",
] as const satisfies readonly TranscriptFormat[];

interface SessionDetailProps {
  sessionId: string;
  onBack: () => void;
}

/** Archived session: insights, one page of transcript, audio and export. */
export const SessionDetail: React.FC<SessionDetailProps> = ({
  sessionId,
  onBack,
}) => {
  const { t, i18n } = useTranslation();
  const player = useSegmentPlayer();
  const [offset, setOffset] = useState(0);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [action, setAction] = useState<
    "export" | "transcript" | "retry" | "folder" | null
  >(null);
  const [transcriptFormat, setTranscriptFormat] =
    useState<TranscriptFormat>("markdown");
  const [actionError, setActionError] = useState<string | null>(null);
  const [exportPath, setExportPath] = useState<string | null>(null);
  const transcriptRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    try {
      const next = await unwrap(
        commands.getMeetingDetail(sessionId, offset, SEGMENT_PAGE_SIZE),
      );
      setDetail(next);
      setLoadError(null);
    } catch (e) {
      setLoadError(errorMessage(e));
    }
  }, [sessionId, offset]);

  useEffect(() => {
    load();
  }, [load]);

  const hasQueued =
    detail?.segments.some(
      (segment) => segment.text === null && segment.asr_error === null,
    ) ?? false;

  // Segments transcribed after stop or after a retry fill in on their own.
  useEffect(() => {
    if (!hasQueued) return;
    const timer = setInterval(load, PENDING_REFRESH_MS);
    return () => clearInterval(timer);
  }, [hasQueued, load]);

  const run = async (
    kind: NonNullable<typeof action>,
    task: () => Promise<unknown>,
  ) => {
    setAction(kind);
    setActionError(null);
    try {
      await task();
    } catch (e) {
      setActionError(errorMessage(e));
    } finally {
      setAction(null);
    }
  };

  const exportAudio = () =>
    run("export", async () => {
      setExportPath(await unwrap(commands.exportMeetingAudio(sessionId)));
    });
  const exportTranscript = () =>
    run("transcript", async () => {
      if (!detail) return;
      const path = await unwrap(
        commands.exportMeetingTranscript(sessionId, transcriptFormat, {
          title: `${t("meetings.detail.transcript")} · ${formatSessionDate(detail.session.started_at_ms, i18n.language)}`,
          microphone: t("meetings.track.microphone"),
          system: t("meetings.track.system"),
          pending: t("meetings.transcript.queued"),
          failed: t("meetings.status.failed"),
        }),
      );
      setExportPath(path);
      await revealItemInDir(path);
    });
  const retry = () =>
    run("retry", async () => {
      await unwrap(commands.retryMeetingAsr(sessionId));
      await load();
    });
  const openFolder = () =>
    run("folder", () => unwrap(commands.openMeetingFolder(sessionId)));
  const reveal = (path: string) => run("folder", () => revealItemInDir(path));

  const session = detail?.session;
  const insights = detail?.insights;
  const hasFailures =
    detail?.segments.some(
      (segment) =>
        segment.asr_error !== null || segment.asr_status === "no_speech",
    ) ?? false;
  const pageEnd = offset + (detail?.segments.length ?? 0);

  const goToPage = (nextOffset: number) => {
    setOffset(nextOffset);
    transcriptRef.current?.scrollIntoView({ block: "start" });
  };

  const pager = detail && (offset > 0 || detail.has_more) && (
    <div className="flex items-center gap-2">
      <span className="text-xs tabular-nums text-text/60">
        {t("meetings.detail.range", { from: offset + 1, to: pageEnd })}
      </span>
      <Button
        variant="ghost"
        size="sm"
        disabled={offset === 0}
        onClick={() => goToPage(Math.max(0, offset - SEGMENT_PAGE_SIZE))}
      >
        {t("meetings.detail.previous")}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={!detail.has_more}
        onClick={() => goToPage(offset + SEGMENT_PAGE_SIZE)}
      >
        {t("meetings.detail.next")}
      </Button>
    </div>
  );

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={onBack}
          className="flex items-center gap-1"
        >
          <ChevronLeft width={16} height={16} />
          {t("meetings.sessions.back")}
        </Button>
        {session && (
          <>
            <span className="text-sm font-medium truncate">
              {formatSessionDate(session.started_at_ms, i18n.language)}
            </span>
            <span className="text-sm tabular-nums text-text/70">
              {formatClock(session.duration_ms)}
            </span>
            <span className={`text-sm ${sessionStatusClass(session.status)}`}>
              {sessionStatusLabel(t, session.status)}
            </span>
          </>
        )}
      </div>

      {loadError && (
        <p className="px-4 text-sm text-red-500">
          {t("meetings.detail.loadError", { error: loadError })}
        </p>
      )}

      {session && (
        <div className="border border-mid-gray/20 rounded-lg divide-y divide-mid-gray/20">
          <p className="px-4 py-2 text-xs text-text/70">
            {t("meetings.detail.exportChannels")}
          </p>
          {session.error && (
            <p className="px-4 py-2 text-sm text-red-500">{session.error}</p>
          )}
          <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] gap-3 px-4 py-2 text-sm">
            <span className="font-medium">
              {t("meetings.track.microphone")}
            </span>
            <span className="truncate text-text/70" title={session.microphone}>
              {session.microphone}
            </span>
            <span className="font-medium">{t("meetings.track.system")}</span>
            <span className="truncate text-text/70" title={session.system}>
              {session.system}
            </span>
          </div>
          {detail?.track_stats
            .filter(
              (stats) =>
                stats.padded_samples > 0 ||
                stats.dropped_samples > 0 ||
                stats.overlapped_samples > 0,
            )
            .map((stats) => (
              <p key={stats.track} className="px-4 py-2 text-xs text-text/70">
                {t("meetings.detail.trackStats", {
                  track: t(
                    stats.track === 0
                      ? "meetings.track.microphone"
                      : "meetings.track.system",
                  ),
                  padding: Math.round(stats.padded_samples / 16),
                  dropped: stats.dropped_samples,
                  overlap: stats.overlapped_samples,
                })}
              </p>
            ))}
          <div className="flex flex-wrap items-center gap-2 px-4 py-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={exportAudio}
              disabled={action !== null}
            >
              {action === "export"
                ? t("meetings.detail.exporting")
                : t("meetings.detail.export")}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              onClick={openFolder}
              disabled={action !== null}
              className="flex items-center gap-1"
            >
              <FolderOpen width={14} height={14} />
              {t("meetings.detail.openFolder")}
            </Button>
            {(hasFailures || session.error) && (
              <Button
                variant="secondary"
                size="sm"
                onClick={retry}
                disabled={action !== null}
              >
                {action === "retry"
                  ? t("meetings.detail.retrying")
                  : t("meetings.detail.retry")}
              </Button>
            )}
          </div>
          {exportPath && (
            <div className="flex items-center gap-2 px-4 py-2 text-sm">
              <span
                className="flex-1 truncate font-mono text-xs select-text cursor-text"
                title={exportPath}
              >
                {exportPath}
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => reveal(exportPath)}
                disabled={action !== null}
              >
                {t("meetings.detail.reveal")}
              </Button>
            </div>
          )}
          {actionError && (
            <p className="px-4 py-2 text-sm text-red-500">{actionError}</p>
          )}
        </div>
      )}

      {insights &&
        (insights.summary ||
          insights.suggestions.length > 0 ||
          insights.error) && (
          <div className="space-y-2">
            <h2 className="px-4 text-xs font-medium text-mid-gray uppercase tracking-wide">
              {t("meetings.ai.title")}
            </h2>
            <div className="border border-mid-gray/20 rounded-lg py-2 space-y-2">
              {insights.error && (
                <p className="px-4 text-sm text-red-500">{insights.error}</p>
              )}
              {insights.summary && (
                <p className="px-4 text-sm whitespace-pre-wrap select-text cursor-text">
                  {insights.summary}
                </p>
              )}
              {insights.suggestions.map((suggestion, index) => (
                <div
                  key={`${suggestion.at_ms}-${index}`}
                  className="grid grid-cols-[4.5rem_minmax(0,1fr)] gap-2 px-4 text-sm"
                >
                  <span className="text-xs text-text/50 tabular-nums pt-0.5">
                    {formatClock(suggestion.at_ms)}
                  </span>
                  <span className="select-text cursor-text">
                    {suggestion.text}
                  </span>
                </div>
              ))}
              {insights.provider && (
                <p className="px-4 text-xs text-text/50">
                  {`${insights.provider} · ${insights.model}`}
                </p>
              )}
            </div>
          </div>
        )}

      {session && detail && (
        <div ref={transcriptRef} className="space-y-2 scroll-mt-4">
          <div className="px-4 flex flex-wrap items-center gap-2">
            <h2 className="flex-1 text-xs font-medium text-mid-gray uppercase tracking-wide">
              {t("meetings.detail.transcript")}
            </h2>
            <label>
              <span className="sr-only">
                {t("meetings.detail.transcriptFormat")}
              </span>
              <select
                value={transcriptFormat}
                disabled={action !== null}
                onChange={(event) => {
                  const selected = TRANSCRIPT_FORMATS.find(
                    (format) => format === event.currentTarget.value,
                  );
                  if (selected) setTranscriptFormat(selected);
                }}
                className="rounded border border-mid-gray/30 bg-background px-2 py-1 text-sm"
              >
                {TRANSCRIPT_FORMATS.map((format) => (
                  <option key={format} value={format}>
                    {t(`meetings.detail.transcriptFormats.${format}`)}
                  </option>
                ))}
              </select>
            </label>
            <Button
              variant="secondary"
              size="sm"
              onClick={exportTranscript}
              disabled={action !== null || session.status === "recording"}
              title={
                session.status === "recording"
                  ? t("meetings.detail.stopToExport")
                  : undefined
              }
            >
              {action === "transcript"
                ? t("meetings.detail.exporting")
                : t("meetings.detail.exportTranscript")}
            </Button>
            {pager}
          </div>
          <div className="border border-mid-gray/20 rounded-lg">
            {detail.segments.length === 0 ? (
              <p className="px-4 py-3 text-sm text-text/60">
                {t("meetings.detail.empty")}
              </p>
            ) : (
              <TranscriptLines
                session={session}
                segments={detail.segments}
                player={player}
              />
            )}
          </div>
          {pager && <div className="px-4 flex justify-end">{pager}</div>}
        </div>
      )}
    </div>
  );
};
