# Retrograde // Music Player
<div align="center">

![Version](https://img.shields.io/badge/VERSION-2.2.0-4FD6BE?style=for-the-badge&logo=appveyor)
![Platform](https://img.shields.io/badge/PLATFORM-LINUX%20%7C%20.NET%20%2B%20AVALONIA-E8C060?style=for-the-badge&logo=linux&logoColor=white)
![Release](https://img.shields.io/badge/PACKAGE-NATIVE%20AOT-0078D4?style=for-the-badge)
![License](https://img.shields.io/badge/LICENSE-MIT-white?style=for-the-badge)

<p align="center">
  <b>A high-fidelity, local music player built for music lovers with bit-perfect audio, fast library indexing, and a retrofuturistic aesthetic.</b>
</p>

<p align="center">
  <a href="https://github.com/Okarin2507/retrograde/releases"><b>⬇️ Download Latest Release (Linux x64)</b></a>
</p>

</div>

---

## Interface Preview

| **Library Grid View** | **Full Screen Player** |
|:---:|:---:|
| ![Library View](./docs/LibraryGrid.png) | ![Player View](./docs/FullScreen.png) |

---

## About this fork

This is a **Linux-only fork** of [Misaka545/retrograde](https://github.com/Misaka545/retrograde). The Windows code was removed; the original Windows build is on the `windows-legacy` branch and in the upstream project.

* **Native AOT:** Release builds on Linux are Native AOT (no JIT, ~44 MB, ~150 MB PSS after scanning a 350-track library).
* **PipeWire output:** mpv runs in-process through the system `libmpv.so.2` with `ao=pipewire,alsa`.
* **GPU rendering:** Skia runs on the integrated Intel/AMD GPU by default (Mesa GL), with CPU fallback; scrolling and the full-screen animation are several times faster than with CPU rendering. `RETRO_RENDER=software` turns it off.
* **MPRIS:** media keys, `playerctl` and shell widgets (e.g. Noctalia) see title, artist, album, cover, position and can play, pause, skip and seek.
* **Tray:** StatusNotifierItem through Avalonia.

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
* **In-Process mpv:** Playback runs on libmpv inside the app (no helper process), through PipeWire (ALSA as the fallback).
* **Gapless Playback:** The next track is handed to mpv ahead of time, so albums play straight through.
* **Crossfade:** 2 to 12 seconds of equal-power overlap between tracks; an album played in order stays gapless.
* **10-Band Equalizer:** Octave bands from 32 Hz to 16 kHz, ±12 dB, with presets, adjusted live without a stutter.
* **ReplayGain:** OFF / TRACK / ALBUM, from the output menu.
* **Audio Device Routing:** Pick a PipeWire output device in the sound popup's OUTPUT tab; the choice is remembered and applied live.
* **Real-time Codec & Stream Inspector:** Codec, bit depth, sample rate and channels for `FLAC`, `WAV`, `ALAC`, `MP3`, `AAC`, `OGG`, `OPUS` and `WMA`.
* **MPRIS Integration:** Media keys, `playerctl` and shell widgets get title, artist, cover and timeline seek.

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
* **Accent from the Cover:** The seek strip, the playing row and the sung lyric line take the colour of the playing track's cover (tray menu toggle).
* **System Tray & Background Mode:** Keep playing from the tray when the window is closed.

---

## Tech Stack

* **Runtime:** [.NET 10](https://dotnet.microsoft.com/)
* **UI Framework:** [Avalonia 12](https://avaloniaui.net/) with software Skia rendering
* **MVVM:** [CommunityToolkit.Mvvm](https://github.com/CommunityToolkit/dotnet)
* **Audio Engine Backend:** [mpv](https://mpv.io/): libmpv in-process, PipeWire output
* **Desktop integration:** MPRIS over D-Bus (Tmds.DBus.Protocol), StatusNotifierItem tray
* **Metadata Extraction:** [TagLib#](https://github.com/mono/taglib-sharp)
* **Image Processing:** [SkiaSharp](https://github.com/mono/SkiaSharp)
* **Packaging:** Native AOT tarball + `install.sh`

---

## Project Architecture

```
retrograde/
├── assets/                       # App icon and images
├── docs/                         # Developer notes and screenshots
├── packaging/linux/              # install.sh, desktop entry, icon
├── src/
│   ├── Retrograde.App/           # Avalonia UI: views, view models, controls, tray, MPRIS
│   ├── Retrograde.Audio/         # mpv engine (libmpv, device list)
│   └── Retrograde.Core/          # Library store, scanner, covers, playlists, settings, session
├── tools/                        # Dev checks (dotnet run tools/<file>.cs)
└── Retrograde.slnx
```

---

## Installation & Setup

### Linux (x64)
1. Install mpv: `sudo pacman -S mpv` (any distro that ships `libmpv.so.2` works).
2. Download `retrograde-<version>-linux-x64.tar.gz` from [Releases](https://github.com/Okarin2507/retrograde/releases), extract it and run:
   ```bash
   ./install.sh
   ```
   This installs to `~/.local/opt/retrograde`, adds a launcher in `~/.local/bin` and an entry in the app drawer. `./install.sh uninstall` removes it and keeps `~/.retrograde`.

### Build from Source (Developer Setup)

#### Prerequisites
* **.NET 10 SDK**
* `mpv` (libmpv.so.2), plus `clang` and `lld` for the Native AOT publish.

#### Instructions

1. **Clone the Repository**
   ```bash
   git clone https://github.com/Okarin2507/retrograde.git
   cd retrograde
   ```

2. **Run**
   ```bash
   dotnet run --project src/Retrograde.App
   ```

3. **Publish (Native AOT)**
   ```bash
   dotnet publish src/Retrograde.App -c Release -o out
   ```

More details (layout, publish options, test tools) are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

---

## Controls & Shortcuts

| Action | Control / Shortcut |
|---|---|
| **Play / Pause** | `Spacebar` / On-screen Hexagonal Button |
| **Previous / Next Track** | `←` / `→`, media keys (MPRIS), on-screen Skip Buttons |
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

Linux port: Okarin2507
