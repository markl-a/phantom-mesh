// Maps the stable `E_DISPATCH_*` error codes that dispatch.rs throws into
// human-readable Traditional-Chinese guidance for the mobile UI. The Rust
// side deliberately keeps the leading token stable (dispatch.rs §"Stable
// error codes") so the JS layer can pattern-match instead of showing a raw
// code like `E_DISPATCH_AUTH_REQUIRED` to the user.
//
// Some codes carry a suffix after a colon (e.g. `E_DISPATCH_URL_INVALID:
// <reason>`, or an HTTP_STATUS message of `HTTP 401`); we surface that as
// detail while still mapping the leading token to friendly copy.

export interface FriendlyDispatchError {
  /** Short, actionable headline. */
  title: string;
  /** One-line "what to do next", when there's a clear fix. */
  hint?: string;
  /** The raw code/message, kept for bug reports + debugging. */
  raw: string;
}

// Leading-token → friendly copy. Keys are the full stable code; matching is
// on the first whitespace/colon-delimited token so a `code: detail` string
// still resolves. Null-prototype so an arbitrary error string whose leading
// token is `constructor`/`toString` can't resolve to an Object.prototype
// member (would otherwise yield a truthy-but-titleless entry → blank error).
const TABLE: Record<string, { title: string; hint?: string }> = Object.assign(
  Object.create(null) as Record<string, { title: string; hint?: string }>,
  {
  E_DISPATCH_PROMPT_EMPTY: { title: '請先輸入任務內容' },
  E_DISPATCH_PROMPT_TOO_LONG: {
    title: '任務內容過長',
    hint: '上限 8000 字，請縮短後再派送。',
  },
  E_DISPATCH_PROMPT_INVALID: {
    title: '任務內容含無效字元',
    hint: '移除控制字元（如隱藏的 NUL 字元）後再試一次。',
  },
  E_DISPATCH_CAPS_INVALID: { title: '能力標籤格式錯誤' },
  E_DISPATCH_CAPS_TOO_MANY: {
    title: '能力選太多',
    hint: '最多選 3 個能力（capability）。',
  },
  E_DISPATCH_PROVIDER_UNKNOWN: {
    title: '指定的 provider 不存在',
    hint: '改回「(broker picks)」或從清單挑一個已設定的 provider。',
  },
  E_DISPATCH_AUTH_REQUIRED: {
    title: '需要先登入才能派送',
    hint: '到「設定」用 phantommesh.io 登入，拿到 broker token 後再派送任務。',
  },
  E_DISPATCH_URL_INVALID: {
    title: 'coordinator（協調者）網址無效',
    hint: '到「集群」確認 coordinator 的網址設定正確。',
  },
  E_DISPATCH_NETWORK: {
    title: '連不上 coordinator（協調者）',
    hint: '確認網路 / Tailscale 連線，以及 coordinator 是否在線。',
  },
  E_DISPATCH_HTTP_STATUS: {
    title: 'coordinator（協調者）回報錯誤',
    hint: '稍後再試；若持續發生，檢查 coordinator 端的日誌。',
  },
  },
);

/**
 * Resolve a raw dispatch error string into friendly UI copy. Unknown
 * strings pass through as the title verbatim (so we never hide an error we
 * didn't anticipate). Always returns the `raw` string for bug reports.
 */
export function friendlyDispatchError(raw: unknown): FriendlyDispatchError {
  const text = (typeof raw === 'string' ? raw : String(raw ?? '')).trim();
  // Leading token = up to the first space or colon.
  const token = text.split(/[\s:]/, 1)[0] ?? '';
  const mapped = TABLE[token];
  // Detail = anything after the first colon (e.g. URL_INVALID reason,
  // HTTP_STATUS "HTTP 401"). Trim and only keep if non-empty.
  const colonIdx = text.indexOf(':');
  const detail = colonIdx >= 0 ? text.slice(colonIdx + 1).trim() : '';

  if (!mapped) {
    return { title: text || '派送失敗', raw: text };
  }
  const hint =
    detail && detail !== mapped.hint
      ? mapped.hint
        ? `${mapped.hint}（${detail}）`
        : detail
      : mapped.hint;
  return { title: mapped.title, hint, raw: text };
}
