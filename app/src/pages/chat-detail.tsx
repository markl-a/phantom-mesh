// SPEC-31 chat-detail — route /chat; commands_used: (delegated to ConversationView)
//
// Thin mobile page wrapper. All chat logic (history load, send, provider mode,
// reset, daemon start, streaming) lives inside ConversationView, which is fully
// self-managing and takes NO props — so this file only supplies the mobile frame:
// an h-dvh safe-area container + a full-height flex column so ConversationView's
// internal `h-full` + `overflow-y-auto` scroll region resolves against a DEFINITE
// height (h-dvh, not min-h-screen) and scrolls within the viewport — keeping the
// MessageInput bar reachable instead of being pushed off-screen by a growing list.
//
// Props note: ConversationView (../components/conversation/ConversationView) exposes
// a zero-prop default export and self-manages all state, so none are passed.
//
// Bottom inset: ConversationView ends with its own MessageInput bar (the bottom-most
// element of its flex column), NOT a `sticky bottom-0` footer. So we apply the
// safe-area-inset-bottom ONCE here on the scroll frame; there is no sticky footer to
// double the inset against.

import ConversationView from "../components/conversation/ConversationView";

export default function ChatDetail() {
  return (
    <main
      // Root mobile frame: full viewport height + top/side safe-area insets.
      // Bottom inset is handled on the inner flex wrapper below (single source),
      // so it is intentionally NOT repeated here to avoid doubling.
      className="flex h-dvh flex-col bg-spectyn-bg text-spectyn-text
                 pt-[env(safe-area-inset-top)]
                 pl-[env(safe-area-inset-left)]
                 pr-[env(safe-area-inset-right)]"
      role="main"
      aria-label="對話頁面 Chat conversation"
      data-testid="chat-detail"
    >
      {/*
        Full-height flex region. `flex-1 min-h-0` lets ConversationView's own
        `h-full` + internal `overflow-y-auto` message list scroll WITHIN the
        viewport instead of growing the page. `min-h-0` is required so the inner
        scroll container is allowed to shrink below its content height.
        Bottom safe-area padding (>= 0.75rem) keeps the MessageInput bar above the
        home indicator. Dynamic Type friendly: no fixed heights are imposed here.
      */}
      <div className="flex min-h-0 flex-1 flex-col px-3 pt-2 text-base
                      pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <ConversationView />
      </div>
    </main>
  );
}
