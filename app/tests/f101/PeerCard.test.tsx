// F101 · Unit tests — PeerCard component.

import { describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import PeerCard from '../../src/components/mobile/PeerCard';
import type { PeerSummary } from '../../src/stores/clusterPeersStore';

function peer(over: Partial<PeerSummary> = {}): PeerSummary {
  return {
    peer_id: 'p1',
    display_name: 'Phone',
    caps: ['gpu', 'camera'],
    status: 'Online',
    last_seen_unix: 0,
    ...over,
  };
}

describe('<PeerCard />', () => {
  it('renders display_name and the status badge', () => {
    render(
      <PeerCard
        peer={peer()}
        isThisDevice={false}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByText('Phone')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Online');
  });

  it('does not crash when caps is missing (unvalidated backend data)', () => {
    const noCaps = peer({ caps: undefined as unknown as string[] });
    expect(() =>
      render(
        <PeerCard
          peer={noCaps}
          isThisDevice={false}
          isSelected={false}
          onSelect={() => {}}
        />,
      ),
    ).not.toThrow();
    expect(screen.getByText('Phone')).toBeInTheDocument();
  });

  it('shows the "this device" pill only when isThisDevice', () => {
    const { rerender } = render(
      <PeerCard
        peer={peer()}
        isThisDevice={false}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    expect(screen.queryByTestId('this-device-pill')).toBeNull();

    rerender(
      <PeerCard
        peer={peer()}
        isThisDevice={true}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByTestId('this-device-pill')).toBeInTheDocument();
  });

  it('shows "+N more" overflow when caps > 4', () => {
    const many = peer({ caps: ['gpu', 'camera', 'vision', 'gps', 'audio', 'mic'] });
    render(
      <PeerCard
        peer={many}
        isThisDevice={false}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByTestId('caps-overflow')).toHaveTextContent('+2 more');
  });

  it('calls onSelect with the peer_id on click', () => {
    const onSelect = vi.fn();
    render(
      <PeerCard
        peer={peer({ peer_id: 'abc' })}
        isThisDevice={false}
        isSelected={false}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByRole('button'));
    expect(onSelect).toHaveBeenCalledWith('abc');
  });

  it('reflects selected state via aria-pressed', () => {
    const { rerender } = render(
      <PeerCard
        peer={peer()}
        isThisDevice={false}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByRole('button')).toHaveAttribute('aria-pressed', 'false');
    rerender(
      <PeerCard
        peer={peer()}
        isThisDevice={false}
        isSelected={true}
        onSelect={() => {}}
      />,
    );
    expect(screen.getByRole('button')).toHaveAttribute('aria-pressed', 'true');
  });
});
