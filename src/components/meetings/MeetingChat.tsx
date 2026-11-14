import React, { useEffect, useRef } from "react";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import ReactMarkdown, { type Components } from "react-markdown";
import { commands } from "@/bindings";
import { unwrap } from "./meetingFormat";
import type { MeetingController } from "./useMeetingController";

export const MeetingChat: React.FC<{ meeting: MeetingController }> = ({
  meeting: m,
}) => {
  const { tr, t, chat } = m;
  const history = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  useEffect(() => {
    follow.current = true;
  }, [m.active]);
  useEffect(() => {
    if (history.current && follow.current)
      history.current.scrollTop = history.current.scrollHeight;
  }, [m.current?.messages.length, m.current?.live_text, m.active]);
  if (!chat) return null;
  const markdownComponents: Components = {
    a: ({ href, children }) => (
      <a
        href={href}
        onClick={(e) => {
          e.preventDefault();
          if (href && /^https?:\/\//.test(href))
            m.run("link", () => openUrl(href));
          else if (href?.startsWith("/")) m.run("link", () => openPath(href));
        }}
      >
        {children}
      </a>
    ),
  };
  const displayText = (text: string) => {
    try {
      const value = JSON.parse(
        text
          .trim()
          .replace(/^```json\s*/, "")
          .replace(/\s*```$/, ""),
      );
      if (Array.isArray(value.actions))
        return value.actions.map((action: string) => `- ${action}`).join("\n");
      if (value.corrections && typeof value.corrections === "object")
        return tr("correctionsReady");
    } catch {
      /* Ordinary chat messages are Markdown. */
    }
    return text;
  };
  const stop = (id: string) =>
    m.run("agent-stop", () => unwrap(commands.stopMeetingAgent(m.id, id)));
  const images = m.attachments[m.active]?.length ?? 0;
  const selection = m.selectionAttachments[m.active]?.length ?? 0;
  return (
    <section className="meeting-right" aria-label={tr("discussions")}>
      <nav className="meeting-chat-tabs" aria-label={tr("discussions")}>
        {m.workspace?.discussions.map((c) => (
          <div key={c.id} className={m.active === c.id ? "is-active" : ""}>
            <button
              aria-current={m.active === c.id ? "page" : undefined}
              onClick={() => m.setActive(c.id)}
            >
              {c.id === "main" ? tr("principal") : c.title}
              {c.status === "running" && <span className="meeting-agent-dot" />}
              {["stopped", "interrupted", "error"].includes(c.status) && (
                <span> · {tr(c.status)}</span>
              )}
            </button>
            {c.id !== "main" && (
              <button
                aria-label={t("meetings.workspace.stopAgent", {
                  title: c.title,
                })}
                onClick={() => stop(c.id)}
              >
                ×
              </button>
            )}
          </div>
        ))}
        {m.active === "new" && (
          <div className="is-active">
            <button>{tr("newDiscussion")}</button>
          </div>
        )}
        <button
          aria-label={tr("newDiscussion")}
          onClick={() => {
            m.setNewTitle("");
            m.setActive("new");
          }}
        >
          +
        </button>
      </nav>
      <div className="meeting-chat-heading">
        <span>
          {m.active === "main"
            ? tr("mainContext")
            : chat.title || tr("newDiscussion")}
        </span>
        <button onClick={() => m.setTab("prep")}>{tr("context")}</button>
      </div>
      <div
        className="meeting-chat-history"
        role="log"
        aria-label={tr("chatHistory")}
        ref={history}
        onScroll={(e) => {
          const el = e.currentTarget;
          follow.current =
            el.scrollHeight - el.scrollTop - el.clientHeight < 60;
        }}
      >
        {m.current?.messages.map((message, index) => (
          <article
            key={index}
            className={`meeting-message ${message.role === "user" ? "is-user" : ""}`}
          >
            <strong>{tr(message.role === "user" ? "you" : "assistant")}</strong>
            <ReactMarkdown components={markdownComponents}>
              {displayText(message.text)}
            </ReactMarkdown>
          </article>
        ))}
        {chat.live_text && (
          <article className="meeting-message">
            <ReactMarkdown components={markdownComponents}>
              {chat.live_text}
            </ReactMarkdown>
          </article>
        )}
        {chat.status === "running" && (
          <p role="status" className="meeting-agent-status">
            {tr(chat.queued ? "queued" : "working")}
          </p>
        )}
        {chat.error && (
          <p role="alert" className="meeting-error">
            {chat.error}
          </p>
        )}
        {m.active === "new" && (
          <label className="meeting-field">
            {tr("discussionTitle")}
            <input
              value={m.newTitle}
              onChange={(e) => m.setNewTitle(e.target.value)}
            />
          </label>
        )}
        {m.active !== "main" &&
          !["codex", "claude"].includes(chat.provider) && (
            <p className="meeting-hint">{tr("httpToolsHint")}</p>
          )}
      </div>
      {m.tab === "live" && (
        <div
          className="meeting-action-strip"
          aria-label={tr("proposedActions")}
        >
          {m.suggestions.map((suggestion) => (
            <div key={suggestion}>
              <button
                disabled={!!m.busy}
                onClick={() => m.send(suggestion, "message", suggestion)}
                title={suggestion}
              >
                {suggestion}
              </button>
              <button
                aria-label={t("meetings.workspace.dismissSuggestion", {
                  text: suggestion,
                })}
                onClick={() =>
                  m.run("dismiss", () =>
                    m.edit({ kind: "dismiss_suggestion", text: suggestion }),
                  )
                }
              >
                ×
              </button>
            </div>
          ))}
        </div>
      )}
      <div className="meeting-composer-area">
        {(selection > 0 || images > 0) && (
          <div className="meeting-attachments">
            <span>
              {selection
                ? t("meetings.workspace.selected", { count: selection })
                : ""}{" "}
              {images ? t("meetings.workspace.images", { count: images }) : ""}
            </span>
            <button
              aria-label={tr("removeAttachments")}
              onClick={() => {
                m.setAttachments((all) => ({ ...all, [m.active]: [] }));
                m.setSelectionAttachments((all) => ({
                  ...all,
                  [m.active]: [],
                }));
              }}
            >
              ×
            </button>
          </div>
        )}
        <form
          className="meeting-composer"
          onSubmit={(e) => {
            e.preventDefault();
            m.send(m.draft);
          }}
        >
          <textarea
            aria-label={tr("message")}
            placeholder={tr(
              m.active === "main" ? "messagePlaceholder" : "agentPlaceholder",
            )}
            value={m.draft}
            onChange={(e) =>
              m.setDrafts((all) => ({ ...all, [m.active]: e.target.value }))
            }
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                m.send(m.draft);
              }
            }}
          />
          <div className="meeting-model-controls">
            <select
              aria-label={tr("provider")}
              value={chat.provider}
              disabled={chat.status === "running"}
              onChange={(e) => {
                const provider = m.providers.data?.find(
                  (p) => p.id === e.target.value,
                );
                m.setChat({
                  provider: e.target.value,
                  model: provider?.model ?? "",
                });
              }}
            >
              {m.providers.data?.map((p) => (
                <option value={p.id} key={p.id} disabled={!p.available}>
                  {p.label}
                  {!p.available ? ` · ${tr("unavailable")}` : ""}
                </option>
              ))}
            </select>
            <input
              aria-label={tr("model")}
              list="meeting-model-options"
              title={tr("defaultModel")}
              placeholder={
                m.providers.data?.find((p) => p.id === chat.provider)?.model ||
                tr("defaultModel")
              }
              value={chat.model}
              disabled={chat.status === "running"}
              onChange={(e) => m.setChat({ model: e.target.value })}
            />
            <datalist id="meeting-model-options">
              {(
                m.providers.data?.find((p) => p.id === chat.provider)?.models ??
                []
              ).map((model) => (
                <option key={model} value={model} />
              ))}
            </datalist>
            <select
              aria-label={tr("effort")}
              value={chat.effort}
              disabled={chat.status === "running"}
              onChange={(e) => m.setChat({ effort: e.target.value })}
            >
              {["low", "medium", "high"].map((effort) => (
                <option key={effort} value={effort}>
                  {tr(effort)}
                </option>
              ))}
            </select>
            <label title={tr("fastHint")}>
              <input
                type="checkbox"
                checked={chat.fast}
                disabled={chat.status === "running"}
                onChange={(e) => m.setChat({ fast: e.target.checked })}
              />
              {tr("fast")}
            </label>
            <button
              className="meeting-primary meeting-send"
              disabled={!m.draft.trim() || m.busy === "send"}
              aria-label={tr("send")}
            >
              ↑
            </button>
          </div>
        </form>
        {chat.status === "running" && m.active === "main" && (
          <button onClick={() => stop("main")}>{tr("cancelReply")}</button>
        )}
      </div>
    </section>
  );
};
