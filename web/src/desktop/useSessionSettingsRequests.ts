import { type Dispatch, type SetStateAction, useEffect, useState } from "react";
import {
  CLOSE_SESSION_SETTINGS_EVENT,
  OPEN_SESSION_SETTINGS_EVENT,
  pendingSessionSettings,
  sessionSettingsEventMatches,
  sessionSettingsFocusFromEvent,
  type SessionSettingsFocus,
} from "../sessionSettingsOpen";

/** Opens and closes the Run configuration surface on this session's settings
 *  requests, remembering where the cursor starts: Replace or fork (`f`)
 *  lands on its form, everything else on the first Agent field. */
export function useSessionSettingsRequests<Surface extends string>(
  sessionId: string,
  setOpenSurface: Dispatch<SetStateAction<Surface | "config" | null>>,
): { configFocus: SessionSettingsFocus; openConfig: () => void } {
  const [configFocus, setConfigFocus] = useState<SessionSettingsFocus>("agent");
  useEffect(() => {
    const openSettings = (event: Event): void => {
      if (!sessionSettingsEventMatches(event, sessionId)) return;
      setConfigFocus(sessionSettingsFocusFromEvent(event));
      setOpenSurface("config");
    };
    const closeSettings = (event: Event): void => {
      if (sessionSettingsEventMatches(event, sessionId)) {
        setOpenSurface((current) => current === "config" ? null : current);
      }
    };
    const pending = pendingSessionSettings(sessionId);
    if (pending) {
      setConfigFocus(pending);
      setOpenSurface("config");
    }
    document.addEventListener(OPEN_SESSION_SETTINGS_EVENT, openSettings);
    document.addEventListener(CLOSE_SESSION_SETTINGS_EVENT, closeSettings);
    return () => {
      document.removeEventListener(OPEN_SESSION_SETTINGS_EVENT, openSettings);
      document.removeEventListener(CLOSE_SESSION_SETTINGS_EVENT, closeSettings);
    };
  }, [sessionId, setOpenSurface]);
  return {
    configFocus,
    openConfig: () => {
      setConfigFocus("agent");
      setOpenSurface("config");
    },
  };
}
