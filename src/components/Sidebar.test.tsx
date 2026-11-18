import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useAppStore } from '@/stores/app';
import { Sidebar } from './Sidebar';

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState({ page: 'library', videos: [] });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialState);
});

describe('Sidebar', () => {
  it('shows the existing TimePin logo with an accessible name and unchanged sizing', () => {
    render(<Sidebar />);

    const logo = screen.getByRole('img', { name: 'TimePin' });
    expect(logo).toHaveAttribute('src', '/logo.png');
    expect(logo).toHaveClass('mb-6', 'h-10', 'w-10', 'rounded-xl', 'object-cover');
  });

  it('keeps all navigation destinations working alongside the logo', () => {
    render(<Sidebar />);

    expect(screen.getByRole('button', { name: '视频库' })).toHaveAttribute('aria-current', 'page');
    for (const [label, page] of [
      ['标注', 'marks'],
      ['插件', 'plugins'],
      ['设置', 'settings'],
      ['视频库', 'library'],
    ]) {
      fireEvent.click(screen.getByRole('button', { name: label }));
      expect(useAppStore.getState().page).toBe(page);
      expect(screen.getByRole('button', { name: label })).toHaveAttribute('aria-current', 'page');
    }
  });
});
