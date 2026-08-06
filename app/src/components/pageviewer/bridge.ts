export const BRIDGE_METHODS: Record<string, string> = {
  page_db_get: "page_db_get",
  page_db_set: "page_db_set",
  page_db_query: "page_db_query",
  send_message: "send_message",
  send_notification: "send_notification",
  get_cluster_status: "get_cluster_status",
};

export interface BridgeMessage {
  spectyn: true;
  id: string;
  method: string;
  args: Record<string, unknown>;
}

export function isBridgeMessage(data: unknown): data is BridgeMessage {
  if (!data || typeof data !== "object") return false;
  const msg = data as Record<string, unknown>;
  return msg.spectyn === true && typeof msg.id === "string" && typeof msg.method === "string";
}
