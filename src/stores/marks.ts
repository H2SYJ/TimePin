import { create } from 'zustand';
import { api } from '@/lib/ipc';
import type { Mark, MarksDocument } from '@/types';

interface MarksApi {
  getMarks: (videoId: string) => Promise<MarksDocument>;
  saveMarks: (videoId: string, marks: Mark[]) => Promise<MarksDocument>;
}

export interface MarksEntry {
  marks: Mark[];
  loadStatus: 'loading' | 'ready' | 'error';
  loadError: string | null;
  saveStatus: 'idle' | 'saving' | 'error';
  saveError: string | null;
  revision: number;
  savedRevision: number;
}

interface MarksState {
  entries: Record<string, MarksEntry>;
  load: (videoId: string) => Promise<void>;
  retrySave: (videoId: string) => void;
  add: (videoId: string, mark: Mark) => void;
  rename: (videoId: string, markId: string, label: string) => void;
  remove: (videoId: string, markId: string) => void;
}

function checkDocument(document: MarksDocument, videoId: string): MarksDocument {
  if (document.schemaVersion !== '1.0.0' || document.videoId !== videoId || !Array.isArray(document.marks)) {
    throw new Error('标注文档版本或视频标识不匹配');
  }
  return document;
}

/**
 * Per-video cache and write queue live outside the page lifecycle. A late load/save
 * can only update its own video; leaving the page never abandons an accepted edit.
 * A failed save retains the latest draft and pauses the queue until explicit retry.
 */
export function createMarksStore(client: MarksApi) {
  const loads = new Map<string, Promise<void>>();
  const saving = new Set<string>();

  return create<MarksState>((set, get) => {
    const update = (videoId: string, transform: (entry: MarksEntry) => MarksEntry) => {
      set((state) => {
        const entry = state.entries[videoId];
        return entry ? { entries: { ...state.entries, [videoId]: transform(entry) } } : state;
      });
    };

    const flush = async (videoId: string) => {
      if (saving.has(videoId)) return;
      saving.add(videoId);
      try {
        while (true) {
          const entry = get().entries[videoId];
          if (!entry || entry.loadStatus !== 'ready' || entry.saveError || entry.revision === entry.savedRevision) return;
          const { revision, marks } = entry;
          update(videoId, (current) => ({ ...current, saveStatus: 'saving' }));
          try {
            const document = checkDocument(await client.saveMarks(videoId, marks), videoId);
            update(videoId, (current) => ({
              ...current,
              // A response for an older snapshot must never replace newer edits.
              marks: current.revision === revision ? document.marks : current.marks,
              savedRevision: revision,
              saveStatus: current.revision === revision ? 'idle' : 'saving',
              saveError: null,
            }));
          } catch (error) {
            update(videoId, (current) => ({ ...current, saveStatus: 'error', saveError: String(error) }));
            return;
          }
        }
      } finally {
        saving.delete(videoId);
      }
    };

    const edit = (videoId: string, transform: (marks: Mark[]) => Mark[]) => {
      const entry = get().entries[videoId];
      // Never overwrite an unknown on-disk document after a failed initial load.
      if (!entry || entry.loadStatus !== 'ready') return;
      const marks = transform(entry.marks);
      if (marks === entry.marks) return;
      update(videoId, (current) => ({
        ...current,
        marks,
        revision: current.revision + 1,
        saveStatus: current.saveError ? 'error' : 'saving',
      }));
      void flush(videoId);
    };

    return {
      entries: {},
      load(videoId) {
        const pending = loads.get(videoId);
        if (pending) return pending;
        if (get().entries[videoId]?.loadStatus === 'ready') return Promise.resolve();
        set((state) => ({
          entries: {
            ...state.entries,
            [videoId]: {
              marks: [], loadStatus: 'loading', loadError: null,
              saveStatus: 'idle', saveError: null, revision: 0, savedRevision: 0,
            },
          },
        }));
        const request = Promise.resolve().then(async () => {
          try {
            const document = checkDocument(await client.getMarks(videoId), videoId);
            update(videoId, (entry) => ({ ...entry, marks: document.marks, loadStatus: 'ready' }));
          } catch (error) {
            update(videoId, (entry) => ({ ...entry, loadStatus: 'error', loadError: String(error) }));
          } finally {
            loads.delete(videoId);
          }
        });
        loads.set(videoId, request);
        return request;
      },
      retrySave(videoId) {
        const entry = get().entries[videoId];
        if (!entry || entry.loadStatus !== 'ready' || entry.revision === entry.savedRevision) return;
        update(videoId, (current) => ({ ...current, saveError: null, saveStatus: 'saving' }));
        void flush(videoId);
      },
      add(videoId, mark) {
        edit(videoId, (marks) => [...marks, mark]);
      },
      rename(videoId, markId, label) {
        const trimmed = label.trim();
        if (!trimmed) return;
        edit(videoId, (marks) => {
          if (!marks.some((mark) => mark.id === markId && mark.label !== trimmed)) return marks;
          return marks.map((mark) => (mark.id === markId ? { ...mark, label: trimmed } : mark));
        });
      },
      remove(videoId, markId) {
        edit(videoId, (marks) => (marks.some((mark) => mark.id === markId)
          ? marks.filter((mark) => mark.id !== markId) : marks));
      },
    };
  });
}

export const useMarksStore = createMarksStore(api);
