import type { Settings } from '@/types';

export type SettingsErrors = Record<string, string>;

export function validateSettingsDraft(settings: Settings): SettingsErrors {
  const errors: SettingsErrors = {};
  const ranges = [
    ['thumbnailQuality', '缩略图质量', 1, 100],
    ['thumbnailMaxDimension', '缩略图最大尺寸', 80, 1920],
    ['maxConcurrentImports', '最大并发导入数', 1, 8],
  ] as const;
  for (const [key, name, min, max] of ranges) {
    if (!Number.isInteger(settings[key]) || settings[key] < min || settings[key] > max) {
      errors[key] = `${name}必须是 ${min}–${max} 之间的整数`;
    }
  }
  settings.customPluginDirs.forEach((directory, index) => {
    if (!directory.trim() || !/^(?:\/|[a-zA-Z]:[\\/]|\\\\[^\\]+[\\/][^\\]+)/.test(directory)) {
      errors[`plugin-${index}`] = '请输入绝对目录路径，例如 D:\\TimePinPlugins 或 /home/user/plugins';
    }
  });
  // pythonPath 若填写必须是绝对路径，且形如可执行文件
  if (settings.pythonPath && settings.pythonPath.trim()) {
    const p = settings.pythonPath.trim();
    if (!/^(?:\/|[a-zA-Z]:[\\/]|\\\\[^\\]+[\\/][^\\]+)/.test(p)) {
      errors.pythonPath = '请输入 Python 解释器的绝对路径';
    }
  }
  return errors;
}

/** 外部设置变化只刷新未编辑字段，保留页面上的其它未保存更改。 */
export function mergeSettingsDraft(draft: Settings, previous: Settings, next: Settings): Settings {
  return Object.fromEntries((Object.keys(next) as (keyof Settings)[]).map((key) => [
    key,
    JSON.stringify(draft[key]) === JSON.stringify(previous[key]) ? next[key] : draft[key],
  ])) as unknown as Settings;
}
