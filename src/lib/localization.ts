import type { LocalizedText } from '@/types';

/**
 * 按回退链解析插件清单中的可本地化文本：
 * `language` -> `"en-US"` -> 映射中第一个条目 -> `""`。
 *
 * - 普通字符串原样返回（向后兼容）。
 * - 映射取 `language` 键；缺失则 `"en-US"`；再缺失则按 JS 对象键序
 *   （即 JSON 文档顺序）取第一个值；空映射 / 未定义返回 `""`。
 */
export function resolveLocalizedText(
  text: LocalizedText | undefined | null,
  language: string,
): string {
  if (text == null) return '';
  if (typeof text === 'string') return text;
  const direct = text[language];
  if (direct !== undefined) return direct;
  const fallbackEn = text['en-US'];
  if (fallbackEn !== undefined) return fallbackEn;
  const first = Object.values(text)[0];
  return first ?? '';
}
