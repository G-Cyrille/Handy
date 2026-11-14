import React from "react";
import { createRoot } from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import {
  commands,
  type MeetingWorkspace,
  type Session,
  type Segment,
  type Discussion,
} from "../../src/bindings";
import { MeetingWindow } from "../../src/components/meetings/MeetingWindow";
import translations from "../../src/i18n/locales/fr/translation.json";
import "../../src/App.css";

await i18n
  .use(initReactI18next)
  .init({ lng: "fr", resources: { fr: { translation: translations } } });
const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value));
const ok = <T,>(data: T) => ({ status: "ok" as const, data: clone(data) });
const discussion = (id: string, title: string): Discussion => ({
  id,
  title,
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
});
const session: Session = {
  id: "42",
  started_at_ms: 1700000000000,
  duration_ms: 24000,
  status: "recording",
  microphone: "Microphone du Mac",
  system: "Meet",
  ai_enabled: false,
  error: null,
};
const workspace: MeetingWorkspace = {
  title: "Point mensuel",
  folder: "Tibria / Interne",
  goal: "Comprendre les résultats de novembre",
  context: "Comparer les inscriptions et les profils terminés.",
  participants: "Alex, Nora, Sam",
  working_directory: "",
  documents: [],
  related_meetings: [],
  entries: [],
  discussions: [discussion("main", "Principal")],
  dismissed_suggestions: [],
  proposed_actions: [
    "Rapport mensuel + Share",
    "Préparer le ticket formulaire",
  ],
  favorites: [],
  corrections: {},
  speakers: {},
  imported_video: null,
  media_error: null,
};
workspace.discussions[0].messages.push({
  role: "assistant",
  text: "Je peux t’aider à préparer ce point puis suivre l’échange.",
  at_ms: 0,
  images: [],
});
const segments: Segment[] = [
  "Quels sont les résultats du mois en cours ?",
  "On a dix-huit nouvelles inscriptions.",
  "Il faudrait comparer avec octobre.",
  "Le formulaire pose encore problème.",
].map((text, i) => ({
  id: i + 1,
  session_id: "42",
  track: i % 2,
  start_sample: i * 80000,
  sample_count: 80000,
  path: "/fixture.wav",
  text,
  asr_error: null,
  asr_status: "complete",
}));
segments.push({
  id: 99,
  session_id: "42",
  track: 0,
  start_sample: 320000,
  sample_count: 80000,
  path: "/fixture.wav",
  text: null,
  asr_error: null,
  asr_status: "no_speech",
});
commands.getMeetingLibrary = async () =>
  ok([
    { session, title: workspace.title, folder: workspace.folder, summary: "" },
  ]);
commands.getMeetingWorkspace = async () => ok(workspace);
commands.getMeetingSnapshot = async () =>
  ok({
    sessions: [session],
    active_id: session.status === "recording" ? "42" : null,
    elapsed_ms: 24000,
    tracks: [],
    capture_error: null,
    pending: null,
    storage_path: "/fixture",
    provider: "",
    model: "",
    backlog_ms: 0,
    buffered_samples: 0,
    asr_last_duration_ms: 0,
    asr_last_lag_ms: 0,
    preferences: { selection: null, ai_enabled: false },
  });
commands.getMeetingDetail = async () =>
  ok({
    session,
    segments,
    has_more: false,
    total_segments: segments.length,
    track_stats: [],
    insights: {
      summary: "",
      suggestions: [],
      error: null,
      provider: "",
      model: "",
    },
  });
commands.meetingProviders = async () => [
  { id: "codex", label: "Codex", available: true, model: "", models: [] },
  { id: "claude", label: "Claude", available: true, model: "", models: [] },
];
commands.editMeetingWorkspace = async (_, edit) => {
  if (edit.kind === "preparation") Object.assign(workspace, edit);
  if (edit.kind === "draft")
    workspace.discussions.find((c) => c.id === edit.chat_id)!.draft = edit.text;
  if (edit.kind === "note")
    workspace.entries.push({
      id: String(Date.now()),
      at_ms: edit.at_ms,
      kind: "note",
      text: edit.text,
      image: null,
    });
  if (edit.kind === "remove_entry")
    workspace.entries = workspace.entries.filter((e) => e.id !== edit.id);
  if (edit.kind === "dismiss_suggestion")
    workspace.dismissed_suggestions.push(edit.text);
  if (edit.kind === "favorites") workspace.favorites = edit.actions;
  return ok(workspace);
};
commands.sendMeetingMessage = async (_, request) => {
  document.body.dataset.provider = request.provider;
  document.body.dataset.requestedModel = request.model;
  let chat = workspace.discussions.find((c) => c.id === request.chat_id);
  if (!chat) {
    chat = discussion(String(Date.now()), request.title);
    workspace.discussions.push(chat);
  }
  chat.messages.push({
    role: "user",
    text: request.text,
    at_ms: 24000,
    images: [],
  });
  chat.status = "running";
  chat.live_text = "[Résultat en cours](/fixture/result.txt)";
  chat.draft = "";
  return ok(workspace);
};
commands.stopMeetingAgent = async (_, id) => {
  workspace.discussions.find((c) => c.id === id)!.status = "stopped";
  return ok(workspace);
};
commands.captureMeetingImage = async (_, scope, at_ms) => {
  workspace.entries.push({
    id: String(Date.now()),
    at_ms,
    kind: "capture",
    text: scope,
    image: "capture.jpg",
  });
  return ok(workspace);
};
commands.stopMeeting = async () => {
  session.status = "completed";
  return ok(null);
};
createRoot(document.getElementById("root")!).render(
  <MeetingWindow sessionId="42" />,
);
