import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ScreenshotCaptureScope } from "@/bindings";
import {
  checkScreenRecordingPermission,
  requestScreenRecordingPermission,
} from "tauri-plugin-macos-permissions-api";
import { Alert } from "../ui/Alert";
import { Dropdown, type DropdownOption } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface ScreenshotContextToggleProps {
  grouped?: boolean;
}

/**
 * Screenshot context toggle for AI post-processing. Turning it on is the one
 * moment Screen Recording permission is requested, so the hotkey itself never
 * surfaces a system prompt. Only rendered on macOS (see PostProcessingSettings).
 */
export const ScreenshotContextToggle: React.FC<ScreenshotContextToggleProps> =
  React.memo(({ grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const enabled = getSetting("screenshot_context_enabled") ?? false;
    const scope = getSetting("screenshot_capture_scope") ?? "current_monitor";
    const [permissionGranted, setPermissionGranted] = useState(true);
    const scopeOptions = useMemo<DropdownOption[]>(
      () => [
        {
          value: "current_monitor",
          label: t(
            "settings.postProcessing.context.screenshot.scope.options.currentMonitor",
          ),
        },
        {
          value: "all_monitors",
          label: t(
            "settings.postProcessing.context.screenshot.scope.options.allMonitors",
          ),
        },
      ],
      [t],
    );

    const refreshPermission = useCallback(async () => {
      try {
        setPermissionGranted(await checkScreenRecordingPermission());
      } catch (error) {
        console.warn("Failed to check Screen Recording permission:", error);
      }
    }, []);

    // Permission is granted in System Settings, outside the app: re-check
    // whenever the window regains focus while the feature is on.
    useEffect(() => {
      if (!enabled) return;
      refreshPermission();
      window.addEventListener("focus", refreshPermission);
      return () => window.removeEventListener("focus", refreshPermission);
    }, [enabled, refreshPermission]);

    const handleChange = async (next: boolean) => {
      if (next) {
        try {
          const granted = await checkScreenRecordingPermission();
          if (!granted) {
            await requestScreenRecordingPermission();
          }
          setPermissionGranted(granted);
        } catch (error) {
          console.warn("Failed to request Screen Recording permission:", error);
        }
      }
      updateSetting("screenshot_context_enabled", next);
    };

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={handleChange}
          isUpdating={isUpdating("screenshot_context_enabled")}
          label={t("settings.postProcessing.context.screenshot.label")}
          description={t(
            "settings.postProcessing.context.screenshot.description",
          )}
          descriptionMode="tooltip"
          grouped={grouped}
        />
        {enabled && (
          <SettingContainer
            title={t("settings.postProcessing.context.screenshot.scope.label")}
            description={t(
              "settings.postProcessing.context.screenshot.scope.description",
            )}
            descriptionMode="tooltip"
            grouped={grouped}
          >
            <Dropdown
              options={scopeOptions}
              selectedValue={scope}
              onSelect={(value) =>
                updateSetting(
                  "screenshot_capture_scope",
                  value as ScreenshotCaptureScope,
                )
              }
              disabled={isUpdating("screenshot_capture_scope")}
            />
          </SettingContainer>
        )}
        {enabled && (
          <ToggleSwitch
            checked={getSetting("screenshot_cursor_highlight") ?? false}
            onChange={(value) =>
              updateSetting("screenshot_cursor_highlight", value)
            }
            isUpdating={isUpdating("screenshot_cursor_highlight")}
            label={t(
              "settings.postProcessing.context.screenshot.cursorHighlight.label",
            )}
            description={t(
              "settings.postProcessing.context.screenshot.cursorHighlight.description",
            )}
            descriptionMode="tooltip"
            grouped={grouped}
          />
        )}
        {enabled && (
          <ToggleSwitch
            checked={getSetting("screenshot_focus_highlight") ?? false}
            onChange={(value) =>
              updateSetting("screenshot_focus_highlight", value)
            }
            isUpdating={isUpdating("screenshot_focus_highlight")}
            label={t(
              "settings.postProcessing.context.screenshot.focusHighlight.label",
            )}
            description={t(
              "settings.postProcessing.context.screenshot.focusHighlight.description",
            )}
            descriptionMode="tooltip"
            grouped={grouped}
          />
        )}
        {enabled && !permissionGranted && (
          <Alert variant="warning" contained>
            {t("settings.postProcessing.context.screenshot.permissionMissing")}
          </Alert>
        )}
      </>
    );
  });

ScreenshotContextToggle.displayName = "ScreenshotContextToggle";
