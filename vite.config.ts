import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    host: "0.0.0.0",
    port: 7100,
    strictPort: false,
  },
  build: {
    target: "es2022",
    sourcemap: false,
  },
});
