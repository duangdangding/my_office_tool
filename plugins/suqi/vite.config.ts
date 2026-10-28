import { defineConfig } from "vite";
import { resolve } from "path";

// @ts-expect-error process is supplied by Vite's Node runtime
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  clearScreen: false,
  base: "./",
  appType: "mpa",
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
