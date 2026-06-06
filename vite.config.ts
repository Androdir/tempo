import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config tuned for Tauri. The dev server runs on a fixed port so the
// Rust side (tauri.conf.json -> build.devUrl) can reliably attach to it.
export default defineConfig({
  plugins: [react()],

  // Tauri expects a fixed port and fails if it is not available.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // Don't watch the Rust source tree from Vite.
      ignored: ["**/src-tauri/**"],
    },
  },
});
