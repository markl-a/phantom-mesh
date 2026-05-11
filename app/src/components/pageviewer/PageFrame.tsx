import { useEffect, useRef } from "react";
import { safeInvoke as invoke } from "../../lib/tauri-compat";
import { isBridgeMessage, BRIDGE_METHODS } from "./bridge";

interface Props {
  html: string | null;
}

export default function PageFrame({ html }: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);

  useEffect(() => {
    const handler = async (event: MessageEvent) => {
      if (!isBridgeMessage(event.data)) return;
      const { id, method, args } = event.data;
      const tauriCommand = BRIDGE_METHODS[method];

      if (!tauriCommand) {
        iframeRef.current?.contentWindow?.postMessage(
          { phantomId: id, error: `Unknown method: ${method}` }, "*"
        );
        return;
      }

      try {
        let result: unknown;
        if (method === "send_notification") {
          const { sendNotification } = await import("@tauri-apps/plugin-notification");
          await sendNotification({ title: String(args.title ?? ""), body: String(args.body ?? "") });
          result = { success: true };
        } else {
          result = await invoke(tauriCommand, args);
        }
        iframeRef.current?.contentWindow?.postMessage({ phantomId: id, result }, "*");
      } catch (e) {
        iframeRef.current?.contentWindow?.postMessage({ phantomId: id, error: String(e) }, "*");
      }
    };

    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, []);

  if (!html) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-phantom-muted">
        <p className="text-sm">選擇一個頁面，或請 Agent 生成新頁面</p>
        <p className="text-xs mt-1">例如：「做一個記帳頁面」</p>
      </div>
    );
  }

  return (
    <iframe
      ref={iframeRef}
      srcDoc={html}
      sandbox="allow-scripts allow-forms"
      className="w-full h-full border-0 rounded bg-white"
      title="Generated Page"
    />
  );
}
