import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The port is fixed because `devUrl` in src-tauri/tauri.conf.json points at it.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"] },
  },
  build: { target: "safari14" },
});
