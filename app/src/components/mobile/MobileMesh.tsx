// MobileMesh — merged 節點 (peer list) + 集群 (cluster dispatch settings)
// into a single screen per 2026-05-23 user direction (chat-first + merge
// dispatch UI into one tab). Renders peer status on top, dispatch form
// below, scrollable.

import MobileCluster from "./MobileCluster";
import MobileClusterSettings from "./MobileClusterSettings";

export default function MobileMesh() {
  return (
    <div className="h-full overflow-y-auto">
      <div className="px-4 py-3 border-b border-phantom-border">
        <h2 className="text-sm font-semibold text-phantom-muted uppercase tracking-wide">節點 (online peers)</h2>
      </div>
      <MobileCluster />
      <div className="px-4 py-3 border-t border-phantom-border mt-4">
        <h2 className="text-sm font-semibold text-phantom-muted uppercase tracking-wide">集群派送 (test dispatch)</h2>
      </div>
      <MobileClusterSettings />
    </div>
  );
}
