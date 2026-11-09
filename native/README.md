# Retrograde Native (C# + Avalonia)

Native port of the Retrograde player. It reads the Electron build's data directly: `~/.retrograde/library.json` and the cover cache in `~/.retrograde/covers`. Cover file names are a bit-exact port of `hashName`, so no rescan is needed.

## Layout

| Project | Contents |
|---|---|
| `src/Retrograde.Core` | Models, `LibraryStore` (source-generated JSON, AOT-safe), `CoverPaths.HashName`, settings |
| `src/Retrograde.Audio` | `MpvEngine`: drives `bin/mpv.exe` over its JSON IPC pipe. Shared and WASAPI-exclusive output in one engine. mpv is tied to the app with a Job Object |
| `src/Retrograde.App` | Avalonia 12 UI: frameless window, virtualized album grid, album detail, player bar |
| `tools/*.cs` | Dev checks, run with `dotnet run tools/<file>.cs`: `hashcheck` (cover-name parity), `enginecheck` (muted playback/seek/pause), `importcheck` (dry run of the Electron playlist import), `scancheck` (scans into `RETRO_APPDIR`, never the real library) |

## Run

```bash
dotnet run --project src/Retrograde.App
```

Rendering defaults to software Skia. On this app it measured ~0.6% idle CPU and ~67 MB private memory, versus ~16% and ~286 MB with ANGLE/D3D. Set `RETRO_RENDER=gpu` to use the GPU instead.

## Publish

```bash
dotnet publish src/Retrograde.App -c Release -o out
```

This makes a trimmed, self-contained ReadyToRun build with tiered compilation off: about 57 MB plus mpv. Native AOT is preferred (`-p:PublishAot=true`), but it needs the Visual Studio "Desktop development with C++" workload for the linker.

## Status

Done:
- UI ported 1:1 from the React components: title bar, sidebar (nav, DATA_BANK, visualizer), LIBRARY_DATA grid with the same breakpoints, SEARCH view with tabs, album detail, player bar (hex play button, gradient seek strip, 15-segment volume), toasts, SYSTEM_MANUAL. Lucide icons are drawn by `Controls/Glyph`
- Scanner (`Core/Scanner.cs`, TagLib#): ADD MUSIC (folder/files), drag and drop, or paths on the command line. Same merge rules, cover naming (400px JPEG thumb + full) and library.json layout as electron/scanner.js; the previous file is kept as `library.json.bak`
- Liked songs and playlists in `~/.retrograde/native-collections.json`, seeded once from the Electron build's localStorage (`Core/LevelDbReader.cs` reads its LevelDB directly). FAVORITES page, playlist pages, create/delete playlist, right-click menus (favorite, queue, add to playlist)
- Tray icon (Show Player / Minimize to tray on close / Exit), BACKGROUND_PLAY prompt on first close (seeded from the Electron `retrograde-config.json`), single instance: a second launch shows the window and forwards its paths
- System Media Transport Controls (`Services/MediaControls.cs`): media keys, Windows media card with title/artist/album/cover, play/pause/next/prev and timeline seek. Position updates stop while the window is hidden or minimized
- Queue popup (QUEUE SYSTEM: current, next sequence, remove, clear) and full-screen player (blurred cover backdrop computed once per track, Next in Queue, F / Esc)
- Library browsing: sort and search
- Album detail with disc headers
- Playback queue: next/prev, shuffle, repeat
- Seek, volume, exclusive toggle
- Keyboard shortcuts: Space, ←/→, J/K, Esc

Not yet:
- Album delete / batch SELECT, playlist cover change
- Output device menu
- libmpv in-process engine
