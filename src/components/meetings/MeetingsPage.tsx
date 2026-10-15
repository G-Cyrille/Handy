import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { LiveMeeting } from "./LiveMeeting";
import { MeetingSetup } from "./MeetingSetup";
import { SessionDetail } from "./SessionDetail";
import { SessionList } from "./SessionList";
import { unwrap } from "./meetingFormat";
import { usePolling } from "./usePolling";

type View =
  | { kind: "capture" }
  | { kind: "sessions" }
  | { kind: "detail"; sessionId: string };

/**
 * Meetings section. The capture view shows setup or the running meeting; the
 * archive lists sessions and opens one in detail. Capture runs in the backend,
 * so leaving this page only stops the 1 s snapshot polling.
 */
export const MeetingsPage: React.FC = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<View>({ kind: "capture" });
  const rootRef = useRef<HTMLDivElement>(null);
  const snapshot = usePolling(
    () => unwrap(commands.getMeetingSnapshot()),
    1000,
  );

  // The meeting shortcut opens this section; show the capture view too.
  useEffect(() => {
    const unlisten = listen("meeting-open", () => setView({ kind: "capture" }));
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Each view starts at its top, like switching sidebar sections does.
  useLayoutEffect(() => {
    rootRef.current?.scrollIntoView({ block: "start" });
  }, [view]);

  const data = snapshot.data;
  const activeSession = data?.active_id
    ? data.sessions.find((session) => session.id === data.active_id)
    : undefined;

  const tab = (kind: "capture" | "sessions", label: string) => {
    const active =
      view.kind === kind || (kind === "sessions" && view.kind === "detail");
    return (
      <button
        type="button"
        onClick={() => setView({ kind })}
        className={`px-1 py-1 text-sm font-medium border-b-2 cursor-pointer transition-colors ${
          active
            ? "border-logo-primary text-text"
            : "border-transparent text-text/60 hover:text-text"
        }`}
      >
        {label}
      </button>
    );
  };

  let content: React.ReactNode = null;
  if (view.kind === "detail") {
    content = (
      <SessionDetail
        key={view.sessionId}
        sessionId={view.sessionId}
        onBack={() => setView({ kind: "sessions" })}
      />
    );
  } else if (!data) {
    content = snapshot.error ? null : (
      <p className="px-4 text-sm text-text/60">{t("meetings.loading")}</p>
    );
  } else if (view.kind === "sessions") {
    content = (
      <SessionList
        sessions={data.sessions}
        activeId={data.active_id}
        onOpen={(session) =>
          setView(
            session.id === data.active_id
              ? { kind: "capture" }
              : { kind: "detail", sessionId: session.id },
          )
        }
      />
    );
  } else if (activeSession) {
    content = (
      <LiveMeeting
        key={activeSession.id}
        snapshot={data}
        session={activeSession}
        onStopped={(sessionId) => {
          setView({ kind: "detail", sessionId });
          snapshot.refresh();
        }}
      />
    );
  } else {
    content = <MeetingSetup snapshot={data} onStarted={snapshot.refresh} />;
  }

  return (
    <div
      ref={rootRef}
      className="max-w-3xl w-full mx-auto space-y-4 scroll-mt-4"
    >
      <div className="flex items-center gap-4 px-4 border-b border-mid-gray/20">
        {tab("capture", t("meetings.tabs.capture"))}
        {tab("sessions", t("meetings.tabs.sessions"))}
      </div>
      {snapshot.error && (
        <p className="px-4 text-sm text-red-500">
          {t("meetings.snapshotError", { error: snapshot.error })}
        </p>
      )}
      {!data?.active_id && data?.capture_error && (
        <p role="alert" className="px-4 text-sm text-red-500">
          {data.capture_error}
        </p>
      )}
      {content}
    </div>
  );
};
