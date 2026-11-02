import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import {
  commands,
  type CaptureSources,
  type MeetingSnapshot,
} from "@/bindings";
import { ShortcutInput } from "../settings/ShortcutInput";
import { Button } from "../ui/Button";
import { Select, type SelectOption } from "../ui/Select";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingsGroup } from "../ui/SettingsGroup";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { errorMessage, unwrap } from "./meetingFormat";

/** Select value for "every application" (application_pid: null). */
const ALL_APPS = "all";

interface MeetingSetupProps {
  snapshot: MeetingSnapshot;
  onStarted: () => void;
}

/**
 * Source selection and start. Both sources must be picked explicitly; the
 * last used selection is restored only while those sources still exist.
 */
export const MeetingSetup: React.FC<MeetingSetupProps> = ({
  snapshot,
  onStarted,
}) => {
  const { t } = useTranslation();
  const [sources, setSources] = useState<CaptureSources | null>(null);
  const [sourcesError, setSourcesError] = useState<string | null>(null);
  const [loadingSources, setLoadingSources] = useState(false);
  const [microphoneId, setMicrophoneId] = useState<string | null>(null);
  const [systemSource, setSystemSource] = useState<string | null>(null);
  const [aiEnabled, setAiEnabled] = useState(
    snapshot.preferences.ai_enabled && snapshot.provider !== "",
  );
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);

  const loadSources = useCallback(async () => {
    setLoadingSources(true);
    try {
      const next = await unwrap(commands.listMeetingSources());
      setSources(next);
      setSourcesError(null);
      return next;
    } catch (e) {
      setSourcesError(errorMessage(e));
      return null;
    } finally {
      setLoadingSources(false);
    }
  }, []);

  // Restore the previous selection once, keeping only sources still present.
  const savedSelectionRef = useRef(snapshot.preferences.selection);
  useEffect(() => {
    loadSources().then((next) => {
      const savedSelection = savedSelectionRef.current;
      if (!next || !savedSelection) return;
      if (next.microphones.some((m) => m.id === savedSelection.microphone_id)) {
        setMicrophoneId(savedSelection.microphone_id);
      }
      const pid = savedSelection.application_pid;
      if (pid === null) {
        setSystemSource(ALL_APPS);
      } else if (next.applications.some((app) => app.pid === pid)) {
        setSystemSource(String(pid));
      }
    });
  }, [loadSources]);

  const microphoneOptions = useMemo<SelectOption[]>(
    () =>
      (sources?.microphones ?? []).map((m) => ({ value: m.id, label: m.name })),
    [sources],
  );
  const systemOptions = useMemo<SelectOption[]>(
    () => [
      { value: ALL_APPS, label: t("meetings.setup.allApps") },
      ...(sources?.applications ?? []).map((app) => ({
        value: String(app.pid),
        label: app.name,
      })),
    ],
    [sources, t],
  );

  const microphoneName = sources?.microphones.find(
    (m) => m.id === microphoneId,
  )?.name;
  const applicationName =
    systemSource && systemSource !== ALL_APPS
      ? sources?.applications.find((app) => String(app.pid) === systemSource)
          ?.name
      : undefined;
  const supported = sources?.supported ?? false;
  const canStart =
    supported && microphoneName !== undefined && systemSource !== null;

  const start = async () => {
    if (!microphoneId || !systemSource) return;
    setStarting(true);
    setStartError(null);
    try {
      await unwrap(
        commands.startMeeting(
          {
            microphone_id: microphoneId,
            application_pid:
              systemSource === ALL_APPS ? null : Number(systemSource),
          },
          aiEnabled,
        ),
      );
      onStarted();
    } catch (e) {
      setStartError(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const captureDescription = !microphoneName
    ? null
    : systemSource === ALL_APPS
      ? t("meetings.setup.captureAll", { microphone: microphoneName })
      : applicationName
        ? t("meetings.setup.captureApp", {
            microphone: microphoneName,
            app: applicationName,
          })
        : null;

  const aiConfigured = snapshot.provider !== "";

  return (
    <div className="space-y-6">
      <SettingsGroup title={t("meetings.setup.title")}>
        <SettingContainer
          title={t("meetings.setup.microphone")}
          description={t("meetings.setup.microphoneDescription")}
          grouped
        >
          <Select
            className="w-56"
            value={microphoneId}
            options={microphoneOptions}
            placeholder={t("meetings.setup.microphonePlaceholder")}
            isLoading={loadingSources}
            isClearable={false}
            disabled={starting}
            onChange={setMicrophoneId}
          />
        </SettingContainer>
        <SettingContainer
          title={t("meetings.setup.system")}
          description={t("meetings.setup.systemDescription")}
          grouped
        >
          {sources && !supported ? (
            <span className="text-sm text-text/70">
              {t("meetings.setup.unsupported")}
            </span>
          ) : (
            <div className="flex items-center gap-1">
              <Select
                className="w-56"
                value={systemSource}
                options={systemOptions}
                placeholder={t("meetings.setup.systemPlaceholder")}
                isLoading={loadingSources}
                isClearable={false}
                disabled={starting}
                onChange={setSystemSource}
              />
              <button
                type="button"
                onClick={loadSources}
                disabled={loadingSources || starting}
                className="p-1.5 rounded-md text-text/50 hover:text-logo-primary cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
                title={t("meetings.setup.refresh")}
                aria-label={t("meetings.setup.refresh")}
              >
                <RefreshCw width={16} height={16} />
              </button>
            </div>
          )}
        </SettingContainer>
        <ToggleSwitch
          label={t("meetings.ai.title")}
          description={
            aiConfigured
              ? t("meetings.ai.description", {
                  provider: `${snapshot.provider} · ${snapshot.model}`,
                })
              : t("meetings.ai.unconfigured")
          }
          descriptionMode="inline"
          checked={aiEnabled}
          onChange={setAiEnabled}
          disabled={!aiConfigured || starting}
          grouped
        />
        <ShortcutInput shortcutId="meeting" grouped />
        <div className="flex items-center gap-4 px-4 py-3">
          <div className="flex-1 space-y-1 text-sm text-text/70">
            <p>{captureDescription}</p>
            <p>{t("meetings.setup.screenAwake")}</p>
          </div>
          <Button onClick={start} disabled={!canStart || starting}>
            {starting
              ? t("meetings.setup.starting")
              : t("meetings.setup.start")}
          </Button>
        </div>
        {(sourcesError || startError) && (
          <p className="px-4 py-2 text-sm text-red-500">
            {startError
              ? t("meetings.setup.startError", { error: startError })
              : t("meetings.setup.sourcesError", { error: sourcesError })}
          </p>
        )}
      </SettingsGroup>
    </div>
  );
};
