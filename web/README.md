# sllm-web — UI prototype

Paper & Ink — Editorial UI for SnapdragonLLM. Preact + Vite + TypeScript.
No UI framework, no icon font, no business logic.

## Run

```powershell
cd web
npm install
npm run dev
```

- `http://127.0.0.1:5173/` — responsive app (phone or tablet layout by width).
- `http://127.0.0.1:5173/?frame=phone` — 390×844 phone frame.
- `http://127.0.0.1:5173/tablet-preview.html` — 1180×820 tablet at 0.5 scale.

## Build

```powershell
npm run build
```

Current release bundle:

| Asset | Raw | Gzip |
|---|---:|---:|
| JS | 35.0 KB | 13.1 KB |
| CSS | 54.4 KB | 29.1 KB |
| HTML | 0.8 KB | 0.4 KB |
| Fonts | 11 woff2/woff (Newsreader, Inter, Plex Mono) | — |

Fonts are bundled locally via `@fontsource` (Latin + Cyrillic where available);
no Google Fonts at runtime.

## Structure

```
src/
  app.tsx      screens, navigation, demo streaming
  data.ts      demo chats/models/messages
  icons.tsx    inline SVG icons
  styles.css   tokens + components + responsive rules
  main.tsx     entry
```

## Integration

Production shell: `crates/sllm-webapp` (thin Java Activity + system WebView,
Rust `cdylib` behind JNI). UI assets are embedded in the APK and served from
`https://app.local/` via `shouldInterceptRequest` (ES modules need a real
origin). Models come from `Sllm.models()` → `sllm-core` preflight; no HTTP
server, no open ports.

Measured on MI 8: APK 0.36 MB, cold start 399–550 ms, PSS 84–99 MB, IME and
send work. See `docs/design/2026-11-09-editorial-ui.md` and
`docs/plans/2026-11-09-model-runtime.md`.
