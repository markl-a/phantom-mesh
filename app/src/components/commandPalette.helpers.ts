// Pure, framework-free logic for the command palette (指令面板) so it can be
// unit-tested without rendering React or mocking the page store. The
// CommandPalette component imports these; behaviour is unchanged from the
// previous inline implementation.

/** Minimal shape the filter needs — the real CommandItem has more fields. */
export interface FilterableCommand {
  label: string;
}

/**
 * Case-insensitive substring filter over command labels — a faithful extraction
 * of the component's original inline logic: an EMPTY query returns the full list
 * (the palette shows everything before the user types); any non-empty query
 * (including one made only of spaces) is matched literally against each label.
 */
export function filterCommands<T extends FilterableCommand>(
  commands: readonly T[],
  query: string,
): T[] {
  if (query === '') return commands.slice();
  const q = query.toLowerCase();
  return commands.filter((c) => c.label.toLowerCase().includes(q));
}

/**
 * Move the highlighted index by `delta` (e.g. +1 for ArrowDown, -1 for
 * ArrowUp), clamped to the valid range [0, length-1]. With an empty list the
 * selection stays at 0. Never returns an out-of-range index, so callers can
 * index the filtered list safely.
 */
export function moveSelection(current: number, delta: number, length: number): number {
  if (length <= 0) return 0;
  const next = current + delta;
  if (next < 0) return 0;
  if (next > length - 1) return length - 1;
  return next;
}
