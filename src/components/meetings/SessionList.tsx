import React from "react";
import { useTranslation } from "react-i18next";
import type { Session } from "@/bindings";
import {
  formatClock,
  formatSessionDate,
  sessionStatusClass,
  sessionStatusLabel,
} from "./meetingFormat";

interface SessionListProps {
  sessions: Session[];
  activeId: string | null;
  onOpen: (session: Session) => void;
}

/** Archive of every session, newest first, including interrupted ones. */
export const SessionList: React.FC<SessionListProps> = ({
  sessions,
  activeId,
  onOpen,
}) => {
  const { t, i18n } = useTranslation();
  const sorted = [...sessions].sort(
    (a, b) => b.started_at_ms - a.started_at_ms,
  );

  return (
    <div className="border border-mid-gray/20 rounded-lg divide-y divide-mid-gray/20">
      {sorted.length === 0 ? (
        <p className="px-4 py-3 text-sm text-text/60">
          {t("meetings.sessions.empty")}
        </p>
      ) : (
        sorted.map((session) => {
          const status = session.id === activeId ? "recording" : session.status;
          return (
            <button
              key={session.id}
              type="button"
              onClick={() => onOpen(session)}
              className="w-full grid grid-cols-[minmax(0,1fr)_auto_6.5rem] items-center gap-3 px-4 py-2 text-sm text-start hover:bg-mid-gray/10 cursor-pointer"
            >
              <span className="truncate font-medium">
                {formatSessionDate(session.started_at_ms, i18n.language)}
              </span>
              <span className="tabular-nums text-text/70">
                {formatClock(session.duration_ms)}
              </span>
              <span
                className={`text-end truncate ${sessionStatusClass(status)}`}
              >
                {sessionStatusLabel(t, status)}
              </span>
            </button>
          );
        })
      )}
    </div>
  );
};
