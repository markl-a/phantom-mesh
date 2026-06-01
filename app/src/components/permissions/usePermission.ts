// SPEC-33 §15.2 — React hook wrapping the runtime permission plumbing.
//
// Owns the live status + "requesting" spinner + never-ask-again signal for one
// permission kind, so both `PermissionGate` and feature screens (FocusPage,
// food capture, the settings panel) drive the OS dialog the same way.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  checkPermission,
  requestPermission,
  openAppSettings,
  type PermissionKind,
  type PermissionStatus,
} from "../../lib/permissions";

export interface UsePermissionState {
  /** `unknown` until the first async check resolves. */
  status: PermissionStatus | "unknown";
  neverAskAgain: boolean;
  requesting: boolean;
  /** Trigger the OS dialog; resolves to the new status. */
  request: () => Promise<PermissionStatus>;
  /** Deep-link to OS app settings; resolves false if no bridge handled it. */
  openSettings: () => Promise<boolean>;
  /** Re-read status without prompting (e.g. after returning from settings). */
  refresh: () => void;
}

export function usePermission(kind: PermissionKind): UsePermissionState {
  const [status, setStatus] = useState<PermissionStatus | "unknown">("unknown");
  const [neverAskAgain, setNeverAskAgain] = useState(false);
  const [requesting, setRequesting] = useState(false);
  const mounted = useRef(true);

  const refresh = useCallback(() => {
    void checkPermission(kind).then((s) => {
      if (mounted.current) setStatus(s);
    });
  }, [kind]);

  useEffect(() => {
    mounted.current = true;
    refresh();
    return () => {
      mounted.current = false;
    };
  }, [refresh]);

  // Re-check when the app regains focus — the user may have toggled the
  // permission in system settings and come back.
  useEffect(() => {
    const onVisible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [refresh]);

  const request = useCallback(async (): Promise<PermissionStatus> => {
    setRequesting(true);
    try {
      const res = await requestPermission(kind);
      if (mounted.current) {
        setStatus(res.status);
        setNeverAskAgain(res.neverAskAgain);
      }
      return res.status;
    } finally {
      if (mounted.current) setRequesting(false);
    }
  }, [kind]);

  const openSettings = useCallback(() => openAppSettings(), []);

  return { status, neverAskAgain, requesting, request, openSettings, refresh };
}
