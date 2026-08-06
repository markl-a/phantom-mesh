// F101 · Unit tests — PeerBadge component + colour mapping.

import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import PeerBadge, { badgeStyleFor } from '../../src/components/mobile/peerBadge';
import type { PeerStatus } from '../../src/stores/clusterPeersStore';

describe('badgeStyleFor', () => {
  it('is total over the PeerStatus enum', () => {
    const statuses: PeerStatus[] = ['Online', 'Unhealthy', 'Unknown'];
    for (const s of statuses) {
      const style = badgeStyleFor(s);
      expect(style.label).toBe(s);
      expect(style.dot).toMatch(/^bg-spectyn-/);
      expect(style.text).toMatch(/^text-spectyn-/);
    }
  });

  it('falls back to the Unknown style for an unexpected/undefined status (no crash)', () => {
    // Peer data from the store/backend isn't runtime-validated; a status
    // outside the enum used to fall through and return undefined, crashing
    // PeerBadge + the whole cluster screen (found live on the emulator).
    for (const bad of [undefined, null, 'Offline', 'Connecting', '']) {
      const style = badgeStyleFor(bad as unknown as PeerStatus);
      expect(style).toBeDefined();
      expect(style.dot).toBe('bg-spectyn-muted');
      expect(style.text).toBe('text-spectyn-muted');
    }
  });

  it('maps Online → success, Unhealthy → warning (no red — reserved for dispatch errors)', () => {
    expect(badgeStyleFor('Online').dot).toBe('bg-spectyn-success');
    expect(badgeStyleFor('Unhealthy').dot).toBe('bg-spectyn-warning');
    expect(badgeStyleFor('Unknown').dot).toBe('bg-spectyn-muted');
    // No status should map to the danger colour.
    for (const s of ['Online', 'Unhealthy', 'Unknown'] as PeerStatus[]) {
      expect(badgeStyleFor(s).dot).not.toBe('bg-spectyn-danger');
    }
  });
});

describe('<PeerBadge />', () => {
  it('renders Healthy badge with the Online label', () => {
    render(<PeerBadge status="Online" />);
    expect(screen.getByRole('status')).toHaveTextContent('Online');
  });

  it('renders Unhealthy badge with the warning colour class', () => {
    const { container } = render(<PeerBadge status="Unhealthy" />);
    const dot = container.querySelector('span[aria-hidden="true"]');
    expect(dot?.className).toContain('bg-spectyn-warning');
  });
});
