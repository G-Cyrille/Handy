import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { commands } from "@/bindings";
import { Button } from "../ui/Button";
import {
  errorMessage,
  formatSessionDate,
  sessionStatusLabel,
  unwrap,
} from "./meetingFormat";
import { usePolling } from "./usePolling";
import "./meetings.css";

/** The main window is only a library; a meeting opens in its own window. */
export const MeetingsPage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const library = usePolling(() => unwrap(commands.getMeetingLibrary()), 3000);
  const [search, setSearch] = useState("");
  const [folder, setFolder] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async (operation: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await operation();
      await library.refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    const listener = listen("meeting-open", async () => {
      try {
        const snapshot = await unwrap(commands.getMeetingSnapshot());
        if (snapshot.active_id)
          await unwrap(commands.openMeetingWindow(snapshot.active_id));
        await library.refresh();
      } catch (e) {
        setError(errorMessage(e));
      }
    });
    return () => {
      listener.then((f) => f());
    };
  }, [library.refresh]);
  const create = () =>
    run(async () => {
      const session = await unwrap(
        commands.createMeetingPreparation(
          t("meetings.workspace.newMeeting"),
          folder,
        ),
      );
      await unwrap(commands.openMeetingWindow(session.id));
    });
  const importRecording = () =>
    run(async () => {
      const path = await open({
        multiple: false,
        filters: [
          {
            name: t("meetings.workspace.recordings"),
            extensions: ["mp3", "wav", "m4a", "mp4", "mkv", "mov", "webm"],
          },
        ],
      });
      if (typeof path !== "string") return;
      const session = await unwrap(
        commands.importMeetingRecording(
          path,
          path.split(/[\\/]/).pop() ?? t("meetings.workspace.newMeeting"),
          folder,
        ),
      );
      await unwrap(commands.openMeetingWindow(session.id));
    });
  const folders = [
    ...new Set(
      library.data
        ?.flatMap((m) =>
          m.folder.split("/").map((_, i, parts) =>
            parts
              .slice(0, i + 1)
              .join("/")
              .trim(),
          ),
        )
        .filter(Boolean) ?? [],
    ),
  ].sort();
  const items =
    library.data?.filter(
      (m) =>
        (!folder || m.folder === folder || m.folder.startsWith(`${folder}/`)) &&
        `${m.title} ${m.folder} ${m.summary}`
          .toLocaleLowerCase()
          .includes(search.toLocaleLowerCase()),
    ) ?? [];
  return (
    <div className="meeting-library">
      <div className="meeting-library-head">
        <h1>{t("meetings.title")}</h1>
        <Button variant="secondary" disabled={busy} onClick={importRecording}>
          {t(busy ? "meetings.loading" : "meetings.workspace.import")}
        </Button>
        <Button disabled={busy} onClick={create}>
          {t("meetings.workspace.newMeeting")}
        </Button>
      </div>
      <div className="meeting-library-filters">
        <input
          aria-label={t("meetings.workspace.search")}
          placeholder={t("meetings.workspace.search")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <select
          aria-label={t("meetings.workspace.folder")}
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
        >
          <option value="">{t("meetings.workspace.allFolders")}</option>
          {folders.map((f) => (
            <option key={f}>{f}</option>
          ))}
        </select>
      </div>
      {(error || library.error) && (
        <p role="alert" className="meeting-error">
          {error || library.error}
        </p>
      )}
      {items.length === 0 && <p>{t("meetings.workspace.emptyLibrary")}</p>}
      {items.map((item) => (
        <button
          key={item.session.id}
          className="meeting-library-item"
          onClick={() =>
            run(() => unwrap(commands.openMeetingWindow(item.session.id)))
          }
        >
          <span>
            <strong>
              {item.title ||
                formatSessionDate(item.session.started_at_ms, i18n.language)}
            </strong>
            <span>{item.folder}</span>
            <span>{item.summary}</span>
          </span>
          <span>{sessionStatusLabel(t, item.session.status)}</span>
        </button>
      ))}
    </div>
  );
};
