# Retrograde 2 (C# + Avalonia)

Retrograde 2.0: the native port of the Retrograde player, replacing the Electron build (1.x). It reads the Electron build's data directly: `~/.retrograde/library.json` and the cover cache in `~/.retrograde/covers`. Cover file names are a bit-exact port of `hashName`, so no rescan is needed.

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

A copy started with `RETRO_APPDIR` set to another folder runs as its own instance, next to the everyday one; tests use this with a scratch copy of the data.

Rendering defaults to software Skia. On this app it measured ~0.6% idle CPU and ~67 MB private memory, versus ~16% and ~286 MB with ANGLE/D3D. Set `RETRO_RENDER=gpu` to use the GPU instead.

## Publish

```bash
dotnet publish src/Retrograde.App -c Release -o out
```

This makes a trimmed, self-contained ReadyToRun build with tiered compilation off: about 65 MB plus mpv. Native AOT is preferred (`-p:PublishAot=true`), but it needs the Visual Studio "Desktop development with C++" workload for the linker.

## Installer

```bash
powershell -ExecutionPolicy Bypass -File installer/build.ps1 -Version 2.0.0
```

This publishes the build above and packs it with NSIS into `release/Retrograde Setup <version>.exe`, about 53 MB. `makensis` is found on the PATH, in an NSIS install, or in electron-builder's cache.

The installer:
- installs per user, with no admin prompt, to `%LOCALAPPDATA%\Programs\Retrograde`, the folder of the Electron build it replaces. If the Electron build is still installed there, setup asks for it to be uninstalled first;
- removes the native 1.x previews ("Retrograde Native": folder, shortcuts and uninstall entry);
- adds a Start menu shortcut and, optionally, a desktop shortcut;
- registers an uninstall entry in Windows Settings.

Uninstalling removes only the program folder. The library in `%USERPROFILE%\.retrograde` is shared with the Electron build, so it stays.

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
- SELECT OUTPUT menu: lists mpv's WASAPI devices, remembered in native-settings.json, switched without stopping playback
- Album delete (album page ⋯, right-click DELETE_ALBUM, or SELECT mode for several at once). library.json is rewritten with fields this build does not know kept intact, and the previous file stays as `library.json.bak`
- Playlist cover: CHANGE_COVER / REMOVE_COVER. Files are named `native_pl_*` so covers of playlists imported from Electron are never touched
- Gapless playback: the next track is handed to mpv ahead of time (`loadfile … append`) and mpv runs straight into it. Repeat-one queues the same file behind itself
- ReplayGain (OFF / TRACK / ALBUM) under the SELECT OUTPUT menu, saved in native-settings.json
- Shuffle plays every track of the queue once per round in random order. Prev walks back through what was played, and the queue views list the real upcoming order
- Session restore: queue, current track, position, shuffle and repeat come back paused on the next start (`native-session.json`, `native-queue.json`)
- Rescan (F5, or "Rescan library" in the tray menu) goes over the added folders and every album folder:
  - New files are added.
  - Files whose size or date changed are read again.
  - Missing files are counted. They are removed only after a confirmation, since a moved folder or an offline drive looks the same.
- Album grouping:
  - Files with the same album name in the same folder (a "Disc N" folder counts as its parent) are always one album, whatever their artist tags say. EPs and compilations often tag every track with its own singer as album artist.
  - Across folders, albums that share a name are told apart by the album-artist tag, or by folder and artist when there is none. A second "Greatest Hits" is stored as `Greatest Hits (Artist)`, so library.json stays readable by the Electron build.
  - An album whose tracks are by different artists is shown under the artist every track names (e.g. "Wuthering Waves"), else "Various Artists".
  - A rescan joins albums that earlier scans split this way.
- Covers fall back to `cover` / `folder` / `front` image files next to the tracks when nothing is embedded
- Recovery: a crashed mpv restarts on the next Play, missing files are reported and skipped, and a damaged library.json or native-collections.json is restored from its `.bak`. Unexpected errors go to `native-error.log`
- Collapse the sidebar Spotify-style with the button at the left of the title bar or Ctrl+B; the choice is remembered. Collapsed, it is a 60px column of icons and covers with names in tooltips, and the album grid takes the room (up to 7 columns on wide windows)
- Smooth wheel scrolling in the style of Lenis (`Controls/SmoothWheel.cs`): wheel notches add to one target and the list glides there with frame-rate independent exponential easing
- Parity details from the Electron build:
  - PRIVATE_MEMORY_BANK easter egg: type LONETRAIL in Morse on the sidebar visualizer.
  - CHANGE_IMG overlay on playlist covers.
  - A playlist without its own cover shows its first track's cover.
  - Liked Songs has a gradient-and-heart cover, drawn locally rather than loaded from Spotify.
  - Raising the volume while muted unmutes.
  - ADD_TO_QUEUE on an empty queue starts playing.
  - DELETING_DATA overlay.
  - Tray / SYSTEM_MANUAL "IMPORT_ELECTRON_DATA" merges favorites and playlists made in the Electron build later.
- Full-screen player opens like paint or water spreading from the full-screen button, and folds back into it on close:
  - the edge is a smooth, rippling blot, with droplets splashed ahead that the front swallows (and leaves behind on the way back);
  - while it moves, the window and the player are shown as two still pictures taken on the first frame, so a frame is two image copies (about 7 ms at 1500x1000) instead of repainting both live UIs (25-33 ms). The pictures are rendered at the window's device pixels and DPI and drawn from their pixel rectangle, so they match the live UI pixel for pixel and nothing shifts when it takes over;
  - toggling again midway turns the blot around from where it is, with the same pictures;
  - the player is laid out once shortly after startup, so the first opening does not wait for it;
  - while the player covers the window, the equalizers and pulses under it are not animated.
- Space is play/pause wherever the focus is, also when a button was clicked last
- Motion: toasts slide in and out; modals, the queue popup and pages fade in; the startup orrery turns; scan and delete show spinners. The equalizer on the playing track, the spinning disc of a cover-less track and the pulsing dot run on a low-rate shared clock (`Controls/Ticker.cs`, 12 / 4 fps). With software rendering a 60 fps loop costs about 25% of a core, while the low-rate clock costs a few percent. Every looping animation pauses while the window is not in front, like the Electron build's app-idle. Set `RETRO_ANIM=always` to keep them running in the background, for measuring

Not yet:
- libmpv in-process engine
