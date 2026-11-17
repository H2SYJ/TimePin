import { describe, expect, it, vi } from 'vitest';
import { waitFor } from '@testing-library/react';
import { createMarksStore, type MarksEntry } from '@/stores/marks';
import type { Mark, MarksDocument } from '@/types';

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const mark = (id = 'point'): Mark => ({
  id, startTime: 12.3456, endTime: null, label: id, color: '#abcdef',
  confidence: 0.93, sourcePlugin: 'template-match', thumbnailPath: 'evidence/frame.png', metadata: { keep: 'yes' },
});
const document = (videoId: string, marks: Mark[] = []): MarksDocument => ({ schemaVersion: '1.0.0', videoId, marks });
const client = () => ({
  getMarks: vi.fn(async (id: string) => document(id)),
  saveMarks: vi.fn(async (id: string, marks: Mark[]) => document(id, marks)),
});

function entry(store: ReturnType<typeof createMarksStore>, videoId: string): MarksEntry {
  const value = store.getState().entries[videoId];
  if (!value) throw new Error(`missing marks entry for ${videoId}`);
  return value;
}

describe('per-video marks persistence queue', () => {
  it('deduplicates loads and caches a known document', async () => {
    const api = client();
    const store = createMarksStore(api);
    const first = store.getState().load('a');
    const second = store.getState().load('a');
    expect(second).toBe(first);
    await first;
    await store.getState().load('a');
    expect(api.getMarks).toHaveBeenCalledTimes(1);
    expect(entry(store, 'a').loadStatus).toBe('ready');
  });

  it('blocks all writes until a failed load is successfully retried', async () => {
    const api = client();
    api.getMarks.mockRejectedValueOnce(new Error('broken document'));
    const store = createMarksStore(api);
    await store.getState().load('a');
    store.getState().add('a', mark());
    store.getState().rename('a', 'point', 'new');
    store.getState().remove('a', 'point');
    expect(api.saveMarks).not.toHaveBeenCalled();
    expect(entry(store, 'a').loadError).toContain('broken document');
    api.getMarks.mockResolvedValueOnce(document('a', [mark('plugin')]));
    await store.getState().load('a');
    store.getState().add('a', mark('manual'));
    await waitFor(() => expect(entry(store, 'a').saveStatus).toBe('idle'));
    expect(api.saveMarks).toHaveBeenLastCalledWith('a', [mark('plugin'), mark('manual')]);
  });

  it('serializes rapid edits and never replaces a newer draft with an old save response', async () => {
    const api = client();
    const firstSave = deferred<MarksDocument>();
    const lastSave = deferred<MarksDocument>();
    api.saveMarks.mockImplementationOnce(() => firstSave.promise).mockImplementationOnce(() => lastSave.promise);
    const store = createMarksStore(api);
    await store.getState().load('a');
    store.getState().add('a', mark('one'));
    store.getState().add('a', mark('two'));
    store.getState().rename('a', 'two', 'renamed');
    store.getState().remove('a', 'one');
    expect(api.saveMarks).toHaveBeenCalledTimes(1);
    expect(entry(store, 'a').revision).toBe(4);
    firstSave.resolve(document('a', [mark('one')]));
    await waitFor(() => expect(api.saveMarks).toHaveBeenCalledTimes(2));
    const latest = [{ ...mark('two'), label: 'renamed' }];
    expect(entry(store, 'a').marks).toEqual(latest);
    expect(api.saveMarks).toHaveBeenLastCalledWith('a', latest);
    expect(entry(store, 'a').saveStatus).toBe('saving');
    lastSave.resolve(document('a', latest));
    await waitFor(() => expect(entry(store, 'a').savedRevision).toBe(4));
    expect(entry(store, 'a').saveStatus).toBe('idle');
  });

  it('retains failed drafts and retries the newest snapshot, not the failed snapshot', async () => {
    const api = client();
    api.saveMarks.mockRejectedValueOnce(new Error('disk full'));
    const store = createMarksStore(api);
    await store.getState().load('a');
    store.getState().add('a', mark());
    await waitFor(() => expect(entry(store, 'a').saveStatus).toBe('error'));
    store.getState().rename('a', 'point', 'latest');
    store.getState().add('a', mark('second'));
    expect(api.saveMarks).toHaveBeenCalledTimes(1);
    expect(entry(store, 'a').savedRevision).toBe(0);
    expect(entry(store, 'a').saveError).toContain('disk full');
    store.getState().retrySave('a');
    await waitFor(() => expect(entry(store, 'a').saveStatus).toBe('idle'));
    expect(api.saveMarks).toHaveBeenLastCalledWith('a', [{ ...mark(), label: 'latest' }, mark('second')]);
    expect(entry(store, 'a').savedRevision).toBe(3);
  });

  it('keeps out-of-order video loads and concurrent per-video saves isolated', async () => {
    const api = client();
    const loadA = deferred<MarksDocument>();
    const saveA = deferred<MarksDocument>();
    api.getMarks.mockImplementationOnce(() => loadA.promise);
    api.saveMarks.mockImplementationOnce(() => saveA.promise);
    const store = createMarksStore(api);
    const a = store.getState().load('a');
    await store.getState().load('b');
    expect(entry(store, 'b').marks).toEqual([]);
    loadA.resolve(document('a', [mark('a-original')]));
    await a;
    store.getState().add('a', mark('a-draft'));
    store.getState().add('b', mark('b-draft'));
    await waitFor(() => expect(entry(store, 'b').saveStatus).toBe('idle'));
    expect(entry(store, 'a').saveStatus).toBe('saving');
    saveA.resolve(document('a', [mark('a-original'), mark('a-draft')]));
    await waitFor(() => expect(entry(store, 'a').saveStatus).toBe('idle'));
    expect(entry(store, 'b').marks).toEqual([mark('b-draft')]);
  });

  it('preserves plugin metadata on rename and round-trips add/delete across fresh store loads', async () => {
    let disk = [mark('plugin')];
    const api = {
      getMarks: vi.fn(async (id: string) => document(id, disk)),
      saveMarks: vi.fn(async (id: string, marks: Mark[]) => { disk = marks; return document(id, marks); }),
    };
    const store = createMarksStore(api);
    await store.getState().load('a');
    store.getState().rename('a', 'plugin', 'Renamed plugin');
    await waitFor(() => expect(entry(store, 'a').saveStatus).toBe('idle'));
    expect(disk[0]).toEqual({ ...mark('plugin'), label: 'Renamed plugin' });
    store.getState().add('a', mark('temporary'));
    store.getState().remove('a', 'temporary');
    await waitFor(() => expect(entry(store, 'a').savedRevision).toBe(3));
    const freshStore = createMarksStore(api);
    await freshStore.getState().load('a');
    expect(entry(freshStore, 'a').marks).toEqual([{ ...mark('plugin'), label: 'Renamed plugin' }]);
  });

  it('rejects documents for another video, and idle retry cannot claim an active save', async () => {
    const api = client();
    api.getMarks.mockResolvedValueOnce(document('wrong'));
    const store = createMarksStore(api);
    await store.getState().load('a');
    expect(entry(store, 'a').loadStatus).toBe('error');
    await store.getState().load('a');
    store.getState().retrySave('a');
    expect(entry(store, 'a').saveStatus).toBe('idle');
    expect(api.saveMarks).not.toHaveBeenCalled();
  });

  it('can retry a synchronously throwing client without a stale pending load', async () => {
    const api = client();
    api.getMarks.mockImplementationOnce(() => { throw new Error('sync failure'); });
    const store = createMarksStore(api);
    await store.getState().load('a');
    await store.getState().load('a');
    expect(api.getMarks).toHaveBeenCalledTimes(2);
    expect(entry(store, 'a').loadStatus).toBe('ready');
  });
});
