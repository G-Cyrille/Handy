import React, { useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { commands } from "@/bindings";
import { formatClock, unwrap } from "./meetingFormat";
import type { MeetingController } from "./useMeetingController";

export const MeetingTimeline: React.FC<{ meeting: MeetingController }> = ({
  meeting: m,
}) => {
  const { t, tr } = m;
  const timeline = useRef<HTMLDivElement>(null);
  const noteInput = useRef<HTMLTextAreaElement>(null);
  const follow = useRef(true);
  const [frameTime, setFrameTime] = useState("");
  useEffect(() => {
    if (m.offset === null && timeline.current && follow.current)
      timeline.current.scrollTop = timeline.current.scrollHeight;
  }, [
    m.data?.detail.total_segments,
    m.data?.snapshot.pending?.committed,
    m.workspace?.entries.length,
  ]);
  useEffect(() => {
    if (m.note) noteInput.current?.focus();
  }, [m.note?.id, !!m.note]);
  const entries = [
    ...m.segments
      .filter((segment) => segment.asr_status !== "no_speech")
      .map((segment) => ({
        at_ms: segment.start_sample / 16,
        segment,
        entry: null,
      })),
    ...(m.workspace?.entries ?? []).map((entry) => ({
      at_ms: entry.at_ms,
      entry,
      segment: null,
    })),
  ].sort((a, b) => a.at_ms - b.at_ms || (a.entry ? 1 : -1));
  return (
    <>
      <div className="meeting-transcript-options">
        <span>
          {m.selected.size > 0
            ? t("meetings.workspace.selected", { count: m.selected.size })
            : tr("selectHint")}
        </span>
        <select
          aria-label={tr("textVersion")}
          value={m.original ? "original" : "corrected"}
          onChange={(e) => m.setOriginal(e.target.value === "original")}
        >
          <option value="corrected">{tr("corrected")}</option>
          <option value="original">{tr("original")}</option>
        </select>
      </div>
      <div
        className="meeting-timeline"
        ref={timeline}
        onScroll={(e) => {
          const el = e.currentTarget;
          follow.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < 60;
        }}
      >
        {(m.data?.detail.total_segments ?? 0) > 100 && (
          <div className="meeting-pagination">
            <button
              disabled={
                (m.offset ??
                  Math.max(0, m.data!.detail.total_segments - 100)) === 0
              }
              onClick={() =>
                m.setOffset(
                  Math.max(
                    0,
                    (m.offset ?? m.data!.detail.total_segments - 100) - 100,
                  ),
                )
              }
            >
              {tr("older")}
            </button>
            <button
              onClick={() => {
                m.setOffset(null);
                follow.current = true;
              }}
            >
              {tr("latest")}
            </button>
          </div>
        )}
        {entries.map(({ segment, entry }) =>
          segment ? (
            <article
              className={`meeting-segment ${m.selected.has(segment.id) ? "is-selected" : ""}`}
              key={`segment-${segment.id}`}
            >
              <input
                type="checkbox"
                aria-label={t("meetings.workspace.selectPassage", {
                  time: formatClock(segment.start_sample / 16),
                })}
                checked={m.selected.has(segment.id)}
                onChange={() => m.selectSegment(segment.id)}
              />
              <div>
                <div className="meeting-segment-meta">
                  <input
                    className="meeting-speaker"
                    aria-label={t("meetings.workspace.speakerAt", {
                      time: formatClock(segment.start_sample / 16),
                    })}
                    defaultValue={
                      m.workspace?.speakers[String(segment.id)] ??
                      t(
                        segment.track === 0
                          ? "meetings.track.microphone"
                          : "meetings.track.system",
                      )
                    }
                    key={`${segment.id}-${m.workspace?.speakers[String(segment.id)]}`}
                    onBlur={(e) => {
                      if (
                        e.target.value.trim() &&
                        e.target.value !==
                          m.workspace?.speakers[String(segment.id)]
                      )
                        m.run("speaker", () =>
                          m.edit({
                            kind: "speaker",
                            segment_id: String(segment.id),
                            name: e.target.value.trim(),
                          }),
                        );
                    }}
                  />
                  <button
                    aria-label={t("meetings.workspace.playPassage", {
                      time: formatClock(segment.start_sample / 16),
                    })}
                    onClick={() => m.player.toggle(segment)}
                  >
                    {formatClock(segment.start_sample / 16)}{" "}
                    {m.player.playingId === segment.id ? "Ⅱ" : "▷"}
                  </button>
                </div>
                <p>
                  {(!m.original &&
                    m.workspace?.corrections[String(segment.id)]) ||
                    segment.text ||
                    segment.asr_error ||
                    tr("pendingTranscription")}
                </p>
              </div>
            </article>
          ) : (
            entry && (
              <article
                key={entry.id}
                className={`meeting-entry meeting-entry-${entry.kind}`}
              >
                <div>
                  <strong>
                    {tr(entry.kind === "note" ? "myNote" : "capture")}
                  </strong>
                  <time>{formatClock(entry.at_ms)}</time>
                  {entry.kind === "note" && (
                    <button
                      onClick={() =>
                        m.setNote({
                          id: entry.id,
                          text: entry.text,
                          at_ms: entry.at_ms,
                        })
                      }
                    >
                      {tr("edit")}
                    </button>
                  )}
                  <button
                    aria-label={tr("removeEntry")}
                    onClick={() =>
                      m.run("remove-entry", () =>
                        m.edit({ kind: "remove_entry", id: entry.id }),
                      )
                    }
                  >
                    ×
                  </button>
                </div>
                {entry.kind === "note" ? (
                  <p>{entry.text}</p>
                ) : (
                  <>
                    <img
                      alt={t("meetings.workspace.captureAt", {
                        time: formatClock(entry.at_ms),
                      })}
                      src={convertFileSrc(`${m.directory}/${entry.image}`)}
                    />
                    <button
                      onClick={() =>
                        m.setAttachments((all) => ({
                          ...all,
                          [m.active]: [
                            ...new Set([...(all[m.active] ?? []), entry.id]),
                          ],
                        }))
                      }
                    >
                      {tr("attachChat")}
                    </button>
                  </>
                )}
              </article>
            )
          ),
        )}
        {m.recording && m.data?.snapshot.pending?.session_id === m.id && (
          <p className="meeting-pending">
            {m.data.snapshot.pending.committed}{" "}
            <span>{m.data.snapshot.pending.tentative}</span>
          </p>
        )}
        {entries.length === 0 && (
          <p className="meeting-hint">
            {tr(
              m.session?.status === "prepared"
                ? "beforeRecording"
                : "emptyTranscript",
            )}
          </p>
        )}
      </div>
      {m.note && (
        <form
          className="meeting-note-editor"
          onSubmit={(e) => {
            e.preventDefault();
            m.saveNote();
            follow.current = true;
          }}
        >
          <label htmlFor="meeting-note">
            {tr("myNote")} · {formatClock(m.note.at_ms)}
          </label>
          <textarea
            id="meeting-note"
            ref={noteInput}
            value={m.note.text}
            onChange={(e) =>
              m.setNote((note) =>
                note ? { ...note, text: e.target.value } : null,
              )
            }
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                m.saveNote();
              }
            }}
          />
          <div>
            <button type="button" onClick={() => m.setNote(null)}>
              {tr("cancel")}
            </button>
            <button
              className="meeting-primary"
              disabled={!m.note.text.trim() || m.busy === "note"}
            >
              {tr("saveNote")}
            </button>
          </div>
        </form>
      )}
      {m.selected.size > 0 && (
        <div className="meeting-selection">
          <button
            onClick={() =>
              m.setSelectionAttachments((all) => ({
                ...all,
                [m.active]: [...m.selected],
              }))
            }
          >
            {tr("attachSelection")}
          </button>
          <button
            disabled={!!m.busy || m.chat?.status === "running"}
            onClick={() => m.send(tr("correctPrompt"), "correct")}
          >
            {tr("correctSelection")}
          </button>
          <button onClick={() => m.setSelected(new Set())}>
            {tr("clearSelection")}
          </button>
        </div>
      )}
      <div className="meeting-live-toolbar">
        <button onClick={m.openNote} title="⌘ ⇧ N">
          {tr("note")}
        </button>
        <button disabled={!!m.busy} onClick={m.capture} title="⌘ ⇧ C">
          {tr(m.busy === "capture" ? "capturing" : "capture")}
        </button>
        <select
          aria-label={tr("captureScope")}
          value={m.scope}
          onChange={(e) => m.setScope(e.target.value)}
        >
          {["all", "screen", "application", "region"].map((s) => (
            <option key={s} value={s}>
              {tr(`scope_${s}`)}
            </option>
          ))}
        </select>
        <button
          aria-label={tr("sources")}
          onClick={() => m.setModal("sources")}
        >
          {"⋯"}
        </button>
      </div>
      {m.workspace?.imported_video && (
        <form
          className="meeting-video-tools"
          onSubmit={(e) => {
            e.preventDefault();
            const seconds = frameTime
              .split(":")
              .map(Number)
              .reduce((n, part) => n * 60 + part, 0);
            if (Number.isFinite(seconds) && seconds >= 0)
              m.run("frame", () =>
                unwrap(commands.extractMeetingFrame(m.id, seconds * 1000)),
              );
          }}
        >
          <input
            aria-label={tr("videoTime")}
            placeholder="12:34"
            value={frameTime}
            onChange={(e) => setFrameTime(e.target.value)}
          />
          <button disabled={!!m.busy || !frameTime}>
            {tr("extractFrame")}
          </button>
        </form>
      )}
    </>
  );
};
