import type { Mark } from '@/types';

export function manualMark(startTime: number, endTime: number | null): Mark {
  return {
    id: crypto.randomUUID(),
    startTime,
    endTime,
    label: endTime === null ? '标记点' : '标记区间',
    color: null,
    confidence: null,
    sourcePlugin: null,
    thumbnailPath: null,
    metadata: {},
  };
}

export function normalizeInterval(from: number, to: number): { startTime: number; endTime: number } | null {
  if (!Number.isFinite(from) || !Number.isFinite(to) || from === to) return null;
  return { startTime: Math.min(from, to), endTime: Math.max(from, to) };
}

export function formatTime(seconds: number): string {
  const milliseconds = Math.round((Number.isFinite(seconds) ? Math.max(0, seconds) : 0) * 1000);
  const total = Math.floor(milliseconds / 1000);
  return `${String(Math.floor(total / 3600)).padStart(2, '0')}:${String(Math.floor((total % 3600) / 60)).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}.${String(milliseconds % 1000).padStart(3, '0')}`;
}
