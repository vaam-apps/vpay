// Vite configuration for the Tauri checkout example.
//
// Two of the three settings exist because a Tauri app's Rust half boots the
// front end rather than the other way round:
//
//   * `server.port` / `strictPort` — `src-tauri/tauri.conf.json` names
//     `http://localhost:1420` as `build.devUrl`. If vite silently moved to
//     1421 because something else holds 1420, `tauri dev` would open a
//     window on nothing. `strictPort` turns that into a startup failure.
//   * `server.host` — set from `TAURI_DEV_HOST` so that `tauri android dev`
//     and `tauri ios dev` on a physical device can reach the dev server on
//     the LAN. Unset (the default) it binds to localhost, which is right for
//     desktop and for a simulator/emulator.
//
// `clearScreen: false` keeps the Rust compiler's output visible; it is what
// `create-tauri-app` generates and the reason is the same here.
import { defineConfig } from "vite";

const host = process.env["TAURI_DEV_HOST"];

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    ...(host === undefined ? {} : { host }),
  },
  // `dist/`, which `tauri.conf.json`'s `build.frontendDist` points at as
  // `../dist`. Stated rather than defaulted, because the two values have to
  // agree and one of them is in another file.
  build: {
    outDir: "dist",
    // The oldest WebView this has to run in is Android's on an API-21
    // device (`minSdk = 21`). Same reasoning as `tsconfig.json`'s target.
    target: "es2020",
  },
});
