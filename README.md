# Retrograde // Music Player
<div align="center">

![Version](https://img.shields.io/badge/VERSION-2.1.0-4FD6BE?style=for-the-badge&logo=appveyor)
![Platform](https://img.shields.io/badge/PLATFORM-WINDOWS%20%7C%20.NET%20%2B%20AVALONIA-E8C060?style=for-the-badge)
![Release](https://img.shields.io/badge/PACKAGE-.EXE%20INSTALLER-0078D4?style=for-the-badge&logo=windows)
![License](https://img.shields.io/badge/LICENSE-MIT-white?style=for-the-badge)

<p align="center">
  <b>A high-fidelity, local music player built for music lovers with bit-perfect audio, fast library indexing, and a retrofuturistic aesthetic.</b>
</p>

<p align="center">
  <a href="https://github.com/Misaka545/retrograde/releases"><b>⬇️ Download Latest Windows Installer (.exe)</b></a>
</p>

</div>

---

## Interface Preview

| **Library Grid View** | **Full Screen Player** |
|:---:|:---:|
| ![Library View](./docs/LibraryGrid.png) | ![Player View](./docs/FullScreen.png) |

---

## What's New in 2.0

Retrograde 2.0 is a native rewrite in **C# + Avalonia**, replacing the Electron build (1.x). Same interface, same library, a fraction of the footprint:

* **Lighter:** about 0.6% CPU and ~67 MB of memory at idle, with no browser engine underneath.
* **Same data:** reads the 1.x library (`~/.retrograde`) and cover cache as they are, with no rescan, and imports your liked songs and playlists from 1.x on first start.
* **New:** gapless playback, ReplayGain, session restore, a collapsible sidebar, smooth scrolling and a liquid full-screen transition.

The Electron build stays available at the [`v1-electron`](https://github.com/Misaka545/retrograde/tree/v1-electron) tag.

---

## System Features

### Audio Engine
* **In-Process mpv:** Playback runs on libmpv inside the app (no helper process), falling back to `mpv.exe` when the library is missing.
* **Bit-Perfect WASAPI Exclusive Mode:** Bypasses the Windows mixer for uncompressed, bit-perfect, zero-resample output, and can be switched on and off without stopping the music.
* **Gapless Playback:** The next track is handed to mpv ahead of time, so albums play straight through.
* **Crossfade:** 2 to 12 seconds of equal-power overlap between tracks; an album played in order stays gapless.
* **10-Band Equalizer:** Octave bands from 32 Hz to 16 kHz, ±12 dB, with presets, adjusted live without a stutter.
* **ReplayGain:** OFF / TRACK / ALBUM, from the output menu.
* **Hardware Audio Device Routing:** Pick a WASAPI output device in the sound popup's OUTPUT tab; the choice is remembered and applied live.
* **Real-time Codec & Stream Inspector:** Codec, bit depth, sample rate and channels for `FLAC`, `WAV`, `ALAC`, `MP3`, `AAC`, `OGG`, `OPUS` and `WMA`.
* **Windows Media Integration:** Media keys and the Windows media card (title, artist, cover, timeline seek).

### Library & Scanner
* **TagLib# Scanner:** Add folders or files, drag and drop them onto the window, or rescan everything with `F5`. Changed files are re-read, new ones added, and missing files removed only after you confirm.
* **Folder Watching:** New and changed files in the library folders show up by themselves a few seconds after they land (tray menu toggle).
* **Play Counts & History:** Plays are counted after half a track or four minutes; sort albums by *Recently Played* or *Most Played*.
* **Smart Album Grouping:** Multi-disc folders, EPs and compilations with per-track artists stay one album; same-named albums from different artists are kept apart.
* **Cover Art:** Embedded art, or `cover` / `folder` / `front` images next to the tracks, cached as 400px thumbnails plus full-resolution covers.
* **Favorites & Playlists:** Liked Songs, custom playlists with their own covers, and batch album delete.
* **Smart Playlists:** Rules over the library (format, bit depth, sample rate, year, length, plays, last played, date added, favorite, title / artist / album text), matched all or any, with an order and a limit. They follow the library and the play counts by themselves; four examples come built in.
* **Artists:** The library grid lists artists as well as albums. An artist page shows the most played tracks and every album; the artist line of an album page and of the player bar lead there.
* **Tag Editor:** Right-click a track for *Edit Tags* (title, artist, album, album artist, year, track and disc number, cover), or edit a whole album's name, artist, year and cover from its page. Written into the files; the library follows at once.
* **Session Restore:** Queue, current track, position, shuffle and repeat come back on the next start.
* **Queue Editing:** *Add to Queue* lines tracks up after the current one, *Play Next* puts them at the front of that line (in shuffle too); drag rows in the queue to reorder them.
* **Library Shuffle:** Shuffle mixes the whole library around the playing track; turning it off goes back to the album.
* **Self-Healing Data:** A damaged library or playlist file is restored from its backup, and a crashed audio engine restarts on the next Play.

### Interface
* **Retrofuturistic Aesthetics:** High-contrast terminal color scheme with **Teal** (`#4FD6BE`), **Gold** (`#E8C060`) and **Orange** (`#FF6B35`) accents over deep void black.
* **Virtualized Grid & Search:** Smooth with libraries of tens of thousands of tracks.
* **Liquid Full-Screen Player:** Spreads out from the player bar like ink, with a blurred cover backdrop and the upcoming queue.
* **Synced Lyrics:** From a `.lrc` file next to the track or the lyrics tag; the line being sung lights up and a click on a line seeks to it.
* **Collapsible Sidebar:** Shrinks to a column of icons and covers (`Ctrl+B`).
* **Mini Player:** A small always-on-top window with the cover, title, transport and seek strip (`Ctrl+M`, or the button next to full screen).
* **Taskbar Thumbnail Buttons:** Previous / play-pause / next in the window's taskbar preview.
* **Accent from the Cover:** The seek strip, the playing row and the sung lyric line take the colour of the playing track's cover (tray menu toggle).
* **System Tray & Background Mode:** Keep playing from the tray when the window is closed.

---

## Tech Stack

* **Runtime:** [.NET 10](https://dotnet.microsoft.com/)
* **UI Framework:** [Avalonia 12](https://avaloniaui.net/) with software Skia rendering
* **MVVM:** [CommunityToolkit.Mvvm](https://github.com/CommunityToolkit/dotnet)
* **Audio Engine Backend:** [mpv](https://mpv.io/): libmpv in-process, or `mpv.exe` over its JSON IPC protocol
* **Metadata Extraction:** [TagLib#](https://github.com/mono/taglib-sharp)
* **Image Processing:** [SkiaSharp](https://github.com/mono/SkiaSharp)
* **Installer:** [NSIS](https://nsis.sourceforge.io/)

---

## Project Architecture

```
retrograde/
├── assets/                       # App icon and images
├── bin/                          # mpv.exe (not in git, see below)
├── docs/                         # Developer notes and screenshots
├── installer/
│   ├── build.ps1                 # Publish + pack the installer
│   └── Retrograde.nsi            # NSIS script
├── src/
│   ├── Retrograde.App/           # Avalonia UI: views, view models, controls, tray, media controls
│   ├── Retrograde.Audio/         # mpv engine (process, IPC pipe, device list)
│   └── Retrograde.Core/          # Library store, scanner, covers, playlists, settings, session
├── tools/                        # Dev checks (dotnet run tools/<file>.cs)
└── Retrograde.slnx
```

---

## Installation & Setup

### Pre-built Windows Release (.exe)
1. Head over to the [GitHub Releases](https://github.com/Misaka545/retrograde/releases) page.
2. Download the latest installer: **`Retrograde Setup 2.1.0.exe`**.
3. Run it. It installs per user (no admin prompt), and `mpv.exe` is bundled.

**Upgrading from 1.x:** uninstall the Electron build first (Windows Settings > Apps > Retrograde); setup will ask you to if it finds it. Your library, playlists and favorites in `%USERPROFILE%\.retrograde` are kept and picked up by 2.0.

---

### Build from Source (Developer Setup)

#### Prerequisites
* **.NET 10 SDK**
* **Windows 10/11**
* **MPV Binary:** a Windows build of `mpv.exe` (from [mpv.io](https://mpv.io/) or [shinchiro's builds](https://sourceforge.net/projects/mpv-player-windows/files/)) in `bin/` at the repository root.
* **NSIS** (only for the installer)

#### Instructions

1. **Clone the Repository**
   ```bash
   git clone https://github.com/Misaka545/retrograde.git
   cd retrograde
   ```

2. **Add mpv**
   ```
   retrograde/bin/mpv.exe
   ```

3. **Run**
   ```bash
   dotnet run --project src/Retrograde.App
   ```

4. **Build the Installer (.exe)**
   ```bash
   powershell -ExecutionPolicy Bypass -File installer/build.ps1
   ```
   *This publishes a trimmed, self-contained Release build and packs it with NSIS into `release/Retrograde Setup <version>.exe`.*

More details (layout, publish options, test tools) are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

---

## Controls & Shortcuts

| Action | Control / Shortcut |
|---|---|
| **Play / Pause** | `Spacebar` / On-screen Hexagonal Button |
| **Previous / Next Track** | `←` / `→`, `MediaKeys`, on-screen Skip Buttons |
| **Prev Track Rewind** | `Prev` after 3 seconds restarts the current track |
| **Volume Down / Up** | `J` / `K`, or scroll over the volume bars (`1/15` steps) |
| **Mute** | `M` |
| **Shuffle / Repeat** | `Ctrl+S` / `Ctrl+R` |
| **Toggle Fullscreen Player** | `F`, or the expand icon on the player bar |
| **Collapse Sidebar** | `Ctrl+B` |
| **Mini Player** | `Ctrl+M`, or the mini player icon on the player bar |
| **Keyboard Shortcuts** | `?` |
| **Rescan Library** | `F5` |
| **Context Menu** | Right-click any album or track to like, queue, edit tags, delete, or add to playlist |
| **Dismiss Overlays** | `Escape` |

---

## License

This project is licensed under the [MIT License](LICENSE.md).

Copyright (c) 2025 Misaka
