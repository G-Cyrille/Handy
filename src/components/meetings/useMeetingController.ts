import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  commands,
  type Discussion,
  type MeetingWorkspace,
  type MeetingRequest,
  type WorkspaceEdit,
} from "@/bindings";
import { unwrap, errorMessage } from "./meetingFormat";
import { usePolling } from "./usePolling";
import { useSegmentPlayer } from "./useSegmentPlayer";

export type Preparation = Pick<
  MeetingWorkspace,
  | "title"
  | "folder"
  | "goal"
  | "context"
  | "participants"
  | "working_directory"
  | "documents"
  | "related_meetings"
>;
export const useMeetingController = (id: string) => {
  const { t, i18n } = useTranslation();
  const tr = (key: string) => t(`meetings.workspace.${key}`);
  const [offset, setOffset] = useState<number | null>(null);
  const state = usePolling(async () => {
    const [workspace, snapshot, first] = await Promise.all([
      unwrap(commands.getMeetingWorkspace(id)),
      unwrap(commands.getMeetingSnapshot()),
      unwrap(commands.getMeetingDetail(id, 0, 1)),
    ]);
    const detail = await unwrap(
      commands.getMeetingDetail(
        id,
        offset ?? Math.max(0, first.total_segments - 100),
        100,
      ),
    );
    return { workspace, snapshot, detail };
  }, 1000);
  const providers = usePolling(() => commands.meetingProviders(), 30_000);
  const library = usePolling(
    () => unwrap(commands.getMeetingLibrary()),
    30_000,
  );
  const [prep, setPrep] = useState<Preparation | null>(null);
  const [tab, setTab] = useState<"prep" | "live" | null>(null);
  const [active, setActive] = useState("main");
  const [newTitle, setNewTitle] = useState("");
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [modelEdits, setModelEdits] = useState<
    Record<string, Partial<Discussion>>
  >({});
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [attachments, setAttachments] = useState<Record<string, string[]>>({});
  const [selectionAttachments, setSelectionAttachments] = useState<
    Record<string, number[]>
  >({});
  const [note, setNote] = useState<{
    text: string;
    at_ms: number;
    id: string | null;
  } | null>(null);
  const [scope, setScope] = useState("all");
  const [original, setOriginal] = useState(false);
  const [compact, setCompact] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [modal, setModal] = useState<"sources" | "favorites" | null>(null);
  const player = useSegmentPlayer();
  const data = state.data;
  const workspace = data?.workspace;
  const session = data?.detail.session;
  const recording = data?.snapshot.active_id === id;
  const elapsed = recording
    ? (data?.snapshot.elapsed_ms ?? 0)
    : (session?.duration_ms ?? 0);
  const current: Discussion | undefined =
    active === "new"
      ? {
          id: "new",
          title: newTitle,
          provider: "codex",
          model: "",
          effort: "medium",
          fast: false,
          status: "idle",
          draft: "",
          messages: [],
          native_id: null,
          error: null,
          queued: false,
          turns: [],
          live_text: "",
        }
      : workspace?.discussions.find((c) => c.id === active);
  const chat = current ? { ...current, ...modelEdits[active] } : undefined;
  const draft = drafts[active] ?? current?.draft ?? "";
  const segments = data?.detail.segments ?? [];
  const selectedSegments = segments.filter((s) => selected.has(s.id));
  const currentTime =
    selectedSegments.slice(-1)[0]?.start_sample !== undefined
      ? selectedSegments.slice(-1)[0]!.start_sample / 16
      : elapsed;
  const directory = data ? `${data.snapshot.storage_path}/sessions/${id}` : "";
  const initialized = useRef(false);
  const prepRef = useRef(prep);
  prepRef.current = prep;
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (!workspace || !session) return;
    setPrep(
      (prior) =>
        prior ?? {
          title: workspace.title,
          folder: workspace.folder,
          goal: workspace.goal,
          context: workspace.context,
          participants: workspace.participants,
          working_directory: workspace.working_directory,
          documents: workspace.documents,
          related_meetings: workspace.related_meetings,
        },
    );
    setTab(
      (prior) => prior ?? (session.status === "prepared" ? "prep" : "live"),
    );
  }, [workspace, session]);
  useEffect(() => {
    if (workspace?.title)
      getCurrentWindow()
        .setTitle(workspace.title)
        .catch(() => {});
  }, [workspace?.title]);
  const edit = useCallback(
    (change: WorkspaceEdit) =>
      unwrap(commands.editMeetingWorkspace(id, change)),
    [id],
  );
  useEffect(() => {
    if (!prep) return;
    if (!initialized.current) {
      initialized.current = true;
      return;
    }
    const timer = setTimeout(() => {
      edit({ kind: "preparation", ...prep }).catch((e) =>
        setError(errorMessage(e)),
      );
    }, 600);
    return () => clearTimeout(timer);
  }, [prep, edit]);
  // Each discussion owns its draft, independent of the left-hand pane.
  useEffect(() => {
    const timer = setTimeout(
      () =>
        Promise.all(
          Object.entries(drafts)
            .filter(([key]) => key !== "new")
            .map(([chat_id, text]) => edit({ kind: "draft", chat_id, text })),
        ).catch((e) => setError(errorMessage(e))),
      500,
    );
    return () => clearTimeout(timer);
  }, [drafts, edit]);
  const run = useCallback(
    async (name: string, operation: () => Promise<unknown>) => {
      setBusy(name);
      setError(null);
      try {
        await operation();
        await state.refresh();
      } catch (e) {
        if (mounted.current) setError(errorMessage(e));
      } finally {
        if (mounted.current) setBusy(null);
      }
    },
    [state.refresh],
  );
  const send = async (text: string, purpose = "message", title?: string) => {
    if (!chat || !text.trim()) return;
    const mainChat = workspace?.discussions.find((c) => c.id === "main");
    const sourceChat =
      purpose === "message"
        ? chat
        : mainChat
          ? { ...mainChat, ...modelEdits.main }
          : undefined;
    if (!sourceChat) return;
    const target =
      purpose !== "message"
        ? "main"
        : title !== undefined || active === "new"
          ? null
          : active;
    const request: MeetingRequest = {
      chat_id: target,
      title: title ?? (newTitle || text.slice(0, 60)),
      text,
      provider: sourceChat.provider,
      model: sourceChat.model,
      effort: sourceChat.effort,
      fast: sourceChat.fast,
      segment_ids:
        purpose !== "message" || title !== undefined
          ? [...selected]
          : (selectionAttachments[active] ?? []),
      capture_ids: attachments[active] ?? [],
      purpose,
    };
    await run("send", async () => {
      if (prepRef.current)
        await edit({ kind: "preparation", ...prepRef.current });
      const next = await unwrap(commands.sendMeetingMessage(id, request));
      if (target === null) {
        setActive(next.discussions.slice(-1)[0]?.id ?? "main");
        setNewTitle("");
      }
      if (title === undefined) {
        setDrafts((all) => ({ ...all, [active]: "" }));
        setAttachments((all) => ({ ...all, [active]: [] }));
        setSelectionAttachments((all) => ({ ...all, [active]: [] }));
      }
    });
  };
  const propose = () => send(tr("proposePrompt"), "propose");
  const capture = () =>
    run("capture", () =>
      unwrap(commands.captureMeetingImage(id, scope, currentTime)),
    );
  const openNote = () => {
    setTab("live");
    setNote({ text: "", at_ms: currentTime, id: null });
  };
  useEffect(() => {
    const keydown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "j") {
        e.preventDefault();
        if (
          !busy &&
          workspace?.discussions.find((c) => c.id === "main")?.status !==
            "running"
        )
          propose();
      }
      if (
        (e.metaKey || e.ctrlKey) &&
        e.shiftKey &&
        e.key.toLowerCase() === "n"
      ) {
        e.preventDefault();
        openNote();
      }
      if (
        (e.metaKey || e.ctrlKey) &&
        e.shiftKey &&
        e.key.toLowerCase() === "c"
      ) {
        e.preventDefault();
        if (!busy) capture();
      }
      if (e.key === "Escape") {
        setModal(null);
        setNote(null);
        setSelected(new Set());
      }
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  });
  const toggleCompact = () =>
    run("compact", async () => {
      await getCurrentWindow().setSize(
        new LogicalSize(compact ? 1200 : 460, compact ? 850 : 740),
      );
      setCompact(!compact);
    });
  const saveNote = () => {
    if (note?.text.trim())
      run("note", async () => {
        await edit({ kind: "note", ...note });
        setNote(null);
      });
  };
  const selectSegment = (segmentId: number) =>
    setSelected((prior) => {
      const next = new Set(prior);
      if (!next.delete(segmentId)) next.add(segmentId);
      return next;
    });
  const setChat = (field: Partial<Discussion>) =>
    setModelEdits((all) => ({
      ...all,
      [active]: { ...all[active], ...field },
    }));
  const suggestions = [
    ...new Set([
      ...(workspace?.favorites ?? []),
      ...(workspace?.proposed_actions ?? []),
      ...(data?.detail.insights.suggestions.map((s) => s.text) ?? []),
    ]),
  ].filter((s) => !workspace?.dismissed_suggestions.includes(s));
  return {
    id,
    t,
    tr,
    i18n,
    offset,
    setOffset,
    state,
    providers,
    library,
    prep,
    setPrep,
    tab,
    setTab,
    active,
    setActive,
    newTitle,
    setNewTitle,
    drafts,
    setDrafts,
    setChat,
    selected,
    setSelected,
    attachments,
    setAttachments,
    selectionAttachments,
    setSelectionAttachments,
    note,
    setNote,
    scope,
    setScope,
    original,
    setOriginal,
    compact,
    error,
    setError,
    busy,
    modal,
    setModal,
    player,
    data,
    workspace,
    session,
    recording,
    elapsed,
    current,
    chat,
    draft,
    segments,
    currentTime,
    directory,
    edit,
    run,
    send,
    propose,
    capture,
    openNote,
    toggleCompact,
    saveNote,
    selectSegment,
    suggestions,
  };
};
export type MeetingController = ReturnType<typeof useMeetingController>;
