import { describe, it, expect } from 'vitest';
import {
  filterCommands,
  moveSelection,
} from '../../src/components/commandPalette.helpers';

const cmds = [
  { label: 'Go to Tasks' },
  { label: 'Go to Devices' },
  { label: 'Go to Settings' },
];

describe('filterCommands', () => {
  it('returns the full list (copy) for an empty query', () => {
    const out = filterCommands(cmds, '');
    expect(out).toHaveLength(3);
    expect(out).not.toBe(cmds); // a copy, not the same reference
  });

  it('matches a non-empty query literally, spaces included (faithful to original)', () => {
    // every label contains a space, so a single-space query matches all three…
    expect(filterCommands(cmds, ' ')).toHaveLength(3);
    // …but a triple-space query matches none (no label has 3 consecutive spaces)
    expect(filterCommands(cmds, '   ')).toEqual([]);
  });

  it('matches case-insensitively on a label substring', () => {
    expect(filterCommands(cmds, 'DEVICES').map((c) => c.label)).toEqual([
      'Go to Devices',
    ]);
    expect(filterCommands(cmds, 'go to')).toHaveLength(3);
  });

  it('returns an empty array when nothing matches', () => {
    expect(filterCommands(cmds, 'zzz')).toEqual([]);
  });
});

describe('moveSelection', () => {
  it('moves down and clamps at the last index', () => {
    expect(moveSelection(0, 1, 3)).toBe(1);
    expect(moveSelection(2, 1, 3)).toBe(2); // already last → stays
  });

  it('moves up and clamps at zero', () => {
    expect(moveSelection(1, -1, 3)).toBe(0);
    expect(moveSelection(0, -1, 3)).toBe(0); // already first → stays
  });

  it('never returns an out-of-range index after the list shrinks', () => {
    // selection was 2, list filtered down to 1 item → must clamp to 0
    expect(moveSelection(2, 1, 1)).toBe(0);
    expect(moveSelection(2, -1, 1)).toBe(0);
  });

  it('returns 0 for an empty list', () => {
    expect(moveSelection(0, 1, 0)).toBe(0);
    expect(moveSelection(5, -1, 0)).toBe(0);
  });
});
