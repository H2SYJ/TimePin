/// <reference types="vite/client" />

import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath, URL } from 'node:url';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: false,
    hmr: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: (typeof process !== 'undefined' && process.env?.TAURI_ENV_PLATFORM === 'windows')
      ? 'chrome105'
      : 'safari13',
    minify: typeof process !== 'undefined' && !process.env?.TAURI_ENV_DEBUG ? 'esbuild' : false,
    sourcemap: typeof process !== 'undefined' && !!process.env?.TAURI_ENV_DEBUG,
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: './src/test/setup.ts',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});