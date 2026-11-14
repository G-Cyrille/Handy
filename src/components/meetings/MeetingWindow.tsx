import React, { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { commands } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { MeetingSetup } from "./MeetingSetup";
import { MeetingTimeline } from "./MeetingTimeline";
import { MeetingChat } from "./MeetingChat";
import { useMeetingController, type Preparation } from "./useMeetingController";
import { formatClock, formatSessionDate, unwrap } from "./meetingFormat";
import "./meetings.css";

export const MeetingWindow: React.FC<{ sessionId: string }> = ({
  sessionId,
}) => {
  const m = useMeetingController(sessionId);
  const { t, tr, prep, chat, data, session, workspace } = m;
  const [favorites, setFavorites] = useState("");
  if (!data || !prep || !chat || !session)
    return (
      <div className="meeting-window">
        <p role="status">{m.state.error || tr("loading")}</p>
      </div>
    );
  const field = (
    key: keyof Omit<Preparation, "documents" | "related_meetings">,
    multiline = false,
  ) => (
    <label className="meeting-field">
      {tr(key)}
      {multiline ? (
        <textarea
          value={prep[key]}
          onChange={(e) => m.setPrep({ ...prep, [key]: e.target.value })}
        />
      ) : (
        <input
          value={prep[key]}
          onChange={(e) => m.setPrep({ ...prep, [key]: e.target.value })}
        />
      )}
    </label>
  );
  const addDocument = () =>
    m.run("document", async () => {
      const paths = await open({ multiple: true });
      if (paths)
        m.setPrep({
          ...prep,
          documents: [
            ...new Set([
              ...prep.documents,
              ...(typeof paths === "string" ? [paths] : paths),
            ]),
          ],
        });
    });
  const visibleError =
    m.error ||
    m.state.error ||
    session.error ||
    data.snapshot.capture_error ||
    workspace?.media_error;
  return (
    <div className={`meeting-window ${m.compact ? "is-compact" : ""}`}>
      <header className="meeting-header">
        <div className="meeting-path">
          <span>{prep.folder}</span>
          <button
            onClick={() =>
              m.run("folder", () =>
                unwrap(commands.openMeetingFolder(sessionId)),
              )
            }
          >
            {tr("files")}
          </button>
          <button onClick={m.toggleCompact}>
            {tr(m.compact ? "full" : "compact")}
          </button>
        </div>
        <div className="meeting-heading">
          <h1>
            {prep.title ||
              formatSessionDate(session.started_at_ms, m.i18n.language)}
          </h1>
          <span className={m.recording ? "meeting-recording" : ""}>
            {formatClock(m.elapsed)}
          </span>
          {m.recording ? (
            <button
              className="meeting-stop"
              disabled={!!m.busy}
              onClick={() =>
                m.run("stop", () => unwrap(commands.stopMeeting()))
              }
            >
              {tr("stopRecording")}
            </button>
          ) : (
            session.status === "prepared" && (
              <button
                className="meeting-primary"
                onClick={() => m.setModal("sources")}
              >
                {tr("start")}
              </button>
            )
          )}
        </div>
      </header>
      {visibleError && (
        <div role="alert" className="meeting-error">
          {visibleError}
        </div>
      )}
      <main className="meeting-columns">
        <section className="meeting-left" aria-label={tr("meetingContent")}>
          <nav className="meeting-pane-tabs" aria-label={tr("meetingContent")}>
            <button
              aria-current={m.tab === "prep" ? "page" : undefined}
              onClick={() => m.setTab("prep")}
            >
              {tr("preparation")}
            </button>
            <button
              aria-current={m.tab === "live" ? "page" : undefined}
              onClick={() => m.setTab("live")}
            >
              {tr("live")}
            </button>
          </nav>
          {m.tab === "prep" ? (
            <div className="meeting-preparation">
              {field("title")}
              {field("folder")}
              {field("goal", true)}
              {field("participants")}
              {field("context", true)}
              <div className="meeting-field-row">
                {field("working_directory")}
                <button
                  onClick={() =>
                    m.run("directory", async () => {
                      const path = await open({ directory: true });
                      if (typeof path === "string")
                        m.setPrep({ ...prep, working_directory: path });
                    })
                  }
                >
                  {tr("choose")}
                </button>
              </div>
              <h2>{tr("documents")}</h2>
              {prep.documents.map((path) => (
                <div className="meeting-document" key={path}>
                  <button onClick={() => m.run("open", () => openPath(path))}>
                    {path.split(/[\\/]/).pop()}
                  </button>
                  <button
                    aria-label={tr("removeDocument")}
                    onClick={() =>
                      m.setPrep({
                        ...prep,
                        documents: prep.documents.filter((p) => p !== path),
                      })
                    }
                  >
                    ×
                  </button>
                </div>
              ))}
              <button onClick={addDocument}>{tr("addDocument")}</button>
              <h2>{tr("previousMeetings")}</h2>
              {prep.related_meetings.map((id) => (
                <div className="meeting-document" key={id}>
                  <button
                    onClick={() =>
                      m.run("open-meeting", () =>
                        unwrap(commands.openMeetingWindow(id)),
                      )
                    }
                  >
                    {m.library.data?.find((item) => item.session.id === id)
                      ?.title || id}
                  </button>
                  <button
                    aria-label={tr("removeRelated")}
                    onClick={() =>
                      m.setPrep({
                        ...prep,
                        related_meetings: prep.related_meetings.filter(
                          (value) => value !== id,
                        ),
                      })
                    }
                  >
                    ×
                  </button>
                </div>
              ))}
              <select
                aria-label={tr("addPreviousMeeting")}
                value=""
                onChange={(e) =>
                  m.setPrep({
                    ...prep,
                    related_meetings: [
                      ...prep.related_meetings,
                      e.target.value,
                    ],
                  })
                }
              >
                <option value="" disabled>
                  {tr("addPreviousMeeting")}
                </option>
                {m.library.data
                  ?.filter(
                    (item) =>
                      item.session.id !== m.id &&
                      !prep.related_meetings.includes(item.session.id),
                  )
                  .map((item) => (
                    <option key={item.session.id} value={item.session.id}>
                      {item.title ||
                        formatSessionDate(
                          item.session.started_at_ms,
                          m.i18n.language,
                        )}
                    </option>
                  ))}
              </select>
              <p className="meeting-hint">{tr("contextHint")}</p>
            </div>
          ) : (
            <MeetingTimeline meeting={m} />
          )}
        </section>
        <MeetingChat meeting={m} />
      </main>
      <footer className="meeting-footer">
        <span>
          {m.selected.size
            ? t("meetings.workspace.selected", { count: m.selected.size })
            : tr("recentWords")}
        </span>
        <button
          onClick={() => {
            setFavorites(workspace?.favorites.join("\n") ?? "");
            m.setModal("favorites");
          }}
        >
          {tr("favorites")}
        </button>
        {!m.recording && session.status !== "prepared" && (
          <button
            disabled={!!m.busy}
            onClick={() =>
              m.run("export", async () => {
                await unwrap(commands.exportMeetingBundle(sessionId));
                await unwrap(commands.openMeetingFolder(sessionId));
              })
            }
          >
            {tr("export")}
          </button>
        )}
        <button
          className="meeting-primary"
          disabled={
            !!m.busy ||
            workspace?.discussions.find((c) => c.id === "main")?.status ===
              "running"
          }
          onClick={m.propose}
        >
          {tr("propose")} <kbd>{"⌘ J"}</kbd>
        </button>
      </footer>
      {m.modal && (
        <Dialog
          open
          title={tr(m.modal === "sources" ? "sources" : "favorites")}
          closeLabel={tr("close")}
          onOpenChange={(open) => {
            if (!open) m.setModal(null);
          }}
          contentClassName="meeting-dialog-fields"
        >
          {m.modal === "sources" ? (
            session.status === "prepared" ? (
              <MeetingSetup
                snapshot={data.snapshot}
                sessionId={sessionId}
                onStarted={() => {
                  m.setModal(null);
                  m.setTab("live");
                  m.state.refresh();
                }}
              />
            ) : (
              <>
                <p>{session.microphone}</p>
                <p>{session.system}</p>
                <p>{tr("localAudio")}</p>
                {m.recording && (
                  <button
                    aria-pressed={session.ai_enabled}
                    disabled={!!m.busy || !data.snapshot.provider}
                    onClick={() =>
                      m.run("ai", () =>
                        unwrap(commands.setMeetingAi(!session.ai_enabled)),
                      )
                    }
                  >
                    {t("meetings.ai.title")} ·{" "}
                    {t(
                      session.ai_enabled ? "meetings.ai.on" : "meetings.ai.off",
                      {
                        defaultValue: session.ai_enabled
                          ? tr("enabled")
                          : tr("disabled"),
                      },
                    )}
                  </button>
                )}
                <button
                  disabled={!!m.busy}
                  onClick={() =>
                    m.run("retry", () =>
                      unwrap(commands.retryMeetingAsr(sessionId)),
                    )
                  }
                >
                  {t("meetings.detail.retry")}
                </button>
              </>
            )
          ) : (
            <>
              <label className="meeting-field">
                {tr("favoriteHint")}
                <textarea
                  value={favorites}
                  onChange={(e) => setFavorites(e.target.value)}
                />
              </label>
              <button
                className="meeting-primary"
                onClick={() =>
                  m.run("favorites", async () => {
                    await m.edit({
                      kind: "favorites",
                      actions: favorites
                        .split("\n")
                        .map((s) => s.trim())
                        .filter(Boolean),
                    });
                    m.setModal(null);
                  })
                }
              >
                {tr("save")}
              </button>
              <p className="meeting-hint">{tr("shortcuts")}</p>
            </>
          )}
        </Dialog>
      )}
      {m.player.error && (
        <p role="alert" className="meeting-error">
          {m.player.error}
        </p>
      )}
    </div>
  );
};
