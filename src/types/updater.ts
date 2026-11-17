export interface UpdaterConfig {
  schemaVersion: '1.0.0';
  configured: boolean;
}

export interface UpdateInfo {
  currentVersion: string;
  version: string;
  date?: string;
  body?: string;
}

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'available'
  | 'upToDate'
  | 'notConfigured'
  | 'confirming'
  | 'downloading'
  | 'installing'
  | 'installedNeedsRelaunch'
  | 'error'
  | 'desktopOnly';

export interface UpdateProgress {
  downloadedBytes: number;
  totalBytes: number | null;
}
