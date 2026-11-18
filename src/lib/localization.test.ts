import { describe, expect, it } from 'vitest';
import { resolveLocalizedText } from './localization';

describe('resolveLocalizedText', () => {
  it('returns plain strings unchanged regardless of language', () => {
    expect(resolveLocalizedText('hello', 'zh-CN')).toBe('hello');
    expect(resolveLocalizedText('hello', 'en-US')).toBe('hello');
    expect(resolveLocalizedText('', 'zh-CN')).toBe('');
  });

  it('returns undefined/null as empty string', () => {
    expect(resolveLocalizedText(undefined, 'zh-CN')).toBe('');
    expect(resolveLocalizedText(null, 'zh-CN')).toBe('');
  });

  it('resolves requested locale first', () => {
    const text = { 'en-US': 'English', 'zh-CN': '中文' };
    expect(resolveLocalizedText(text, 'zh-CN')).toBe('中文');
    expect(resolveLocalizedText(text, 'en-US')).toBe('English');
  });

  it('falls back to en-US when requested locale is missing', () => {
    const text = { 'en-US': 'English', 'zh-CN': '中文' };
    expect(resolveLocalizedText(text, 'ja-JP')).toBe('English');
  });

  it('falls back to the first map entry (JSON insertion order) when neither locale nor en-US exists', () => {
    const text = { 'zh-CN': '中文', 'ja-JP': '日本語' };
    expect(resolveLocalizedText(text, 'fr-FR')).toBe('中文');
  });

  it('returns empty string for an empty map', () => {
    expect(resolveLocalizedText({}, 'zh-CN')).toBe('');
  });
});
