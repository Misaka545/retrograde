# Developing Retrograde

Retrograde 2.0 is a native C# + Avalonia port of the Electron build (1.x, kept at the `v1-electron` tag and the `electron` branch). It reads the Electron build's data directly: `~/.retrograde/library.json` and the cover cache in `~/.retrograde/covers`. Cover file names are a bit-exact port of `hashName`, so no rescan is needed.

## Layout

| Project | Contents |
|---|---|
| `src/Retrograde.Core` | Models, `LibraryStore` (source-generated JSON, AOT-safe), `CoverPaths.HashName`, settings |
| `src/Retrograde.Audio` | `MpvEngine`: drives mpv in this process through the system `libmpv.so.2` (`LibMpvClient`, which takes mpv's JSON IPC commands and answers in the same message shape) |
| `src/Retrograde.App` | Avalonia 12 UI: frameless window, virtualized album grid, album detail, player bar |
| `tools/*.cs` | Dev checks, run with `dotnet run tools/<file>.cs`: `hashcheck` (cover-name parity), `enginecheck` (muted playback/seek/pause), `importcheck` (dry run of the Electron playlist import), `scancheck` (scans into `RETRO_APPDIR`, never the real library) |

## Run

Needs the .NET 10 SDK and the `mpv` package (`libmpv.so.2`): `sudo pacman -S mpv`.

```bash
dotnet run --project src/Retrograde.App
```

A copy started with `RETRO_APPDIR` set to another folder runs as its own instance, next to the everyday one; tests use this with a scratch copy of the data.

## Linux notes

- Output: `ao=pipewire,alsa`. There is no exclusive mode; the output device is picked in the sound popup's OUTPUT tab.
- Media keys / widgets: MPRIS on the session bus (`Services/Mpris.cs`, name `org.mpris.MediaPlayer2.retrograde`); try `playerctl -p retrograde status`.
- Rendering: Skia runs on the GPU through GLX/EGL (Mesa on the integrated Intel or AMD GPU), falling back to CPU Skia if GL cannot start. `vblank_mode=0` / `__GL_SYNC_TO_VBLANK=0` are set for you (swaps under XWayland otherwise block for up to a second). `RETRO_RENDER=software` forces CPU rendering; `DRI_PRIME=1` would pick the discrete GPU (measured: same speed, ~150 MB more RAM).
- Native Wayland (`RETRO_BACKEND=wayland`, Avalonia.Wayland, experimental) is opt-in and not recommended yet: in tests its frame loop was not paced.
- Benchmarks without input devices: `RETRO_BENCH=scroll` and `RETRO_BENCH=fullscreen RETRO_FSLOG=1` (results in `/tmp/retro-bench.txt` and `/tmp/retro-fs-frames.txt`). On a 131-album library: CPU rendering 110 fps / p95 37 ms while scrolling, GPU rendering p95 2.6 ms; full-screen animation frame 18 ms vs 3 ms.
- Measure memory as PSS: `grep Pss /proc/$(pgrep -n Retrograde)/smaps_rollup`.

## Publish

```bash
dotnet publish src/Retrograde.App -c Release -o out
```

Native AOT, self-contained linux-x64 (needs clang and lld), about 44 MB. Package it with `packaging/linux`: copy `install.sh`, `retrograde.desktop` and `retrograde.png` next to the binary and its two `.so` files, and tar the folder. `install.sh` installs per user (`~/.local/opt/retrograde`, a launcher in `~/.local/bin`, an app drawer entry); `install.sh uninstall` removes it and keeps `~/.retrograde`.

The Windows build of the original project lives on the `windows-legacy` branch; this fork no longer builds it.

## Status

This list is the original project's history. In this Linux-only fork the Windows items are gone: System Media Transport Controls (replaced by MPRIS), taskbar thumbnail buttons, WASAPI exclusive mode and device routing (now PipeWire), the NSIS installer, the `mpv.exe` fallback engine and its Job Object.

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
- Player bar: track info and the extras take equal shares, so the transport is in the middle of the window; when the window is too narrow for that (under about 1080 wide) the extras keep their width and the track info gets the rest (`MainWindow.BalancePlayerBar`). The extras are volume, favorite, queue, sound, mini player and full screen
- Page titles (`Controls/FitTitle`): 60 on up to two lines, else 46 or 36 on up to three, chosen with the TextBlock's own layout at the width it has; past that the third line ends in "…" and the full title is in a tooltip. The header stays about as tall as the cover however long the name
- Sound popup (the sliders button): EQUALIZER tab (equalizer, crossfade) and OUTPUT tab (device, ReplayGain, WASAPI exclusive). A teal dot on the button means WASAPI exclusive is on
- SELECT OUTPUT (OUTPUT tab): lists mpv's WASAPI devices, remembered in native-settings.json, switched without stopping playback
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
  - PRIVATE_MEMORY_BANK easter egg on the sidebar visualizer.
  - CHANGE_IMG overlay on playlist covers.
  - A playlist without its own cover shows its first track's cover.
  - Liked Songs has a gradient-and-heart cover, drawn locally rather than loaded from Spotify.
  - Raising the volume while muted unmutes.
  - ADD_TO_QUEUE on an empty queue starts playing.
  - DELETING_DATA overlay.
- Full-screen player opens like paint or water spreading from the full-screen button, and folds back into it on close:
  - the edge is a smooth, rippling blot, with droplets splashed ahead that the front swallows (and leaves behind on the way back);
  - while it moves, the window and the player are shown as two still pictures taken on the first frame, so a frame is two image copies (about 7 ms at 1500x1000) instead of repainting both live UIs (25-33 ms). The pictures are rendered at the window's device pixels and DPI and drawn from their pixel rectangle, so they match the live UI pixel for pixel and nothing shifts when it takes over;
  - toggling again midway turns the blot around from where it is, with the same pictures;
  - the player is laid out once shortly after startup, so the first opening does not wait for it;
  - while the player covers the window, the equalizers and pulses under it are not animated.
- Space is play/pause wherever the focus is, also when a button was clicked last
- Motion: toasts slide in and out; modals, the queue popup and pages fade in; scan and delete show spinners. There is no startup loading screen: the library loads in a blink. The equalizer on the playing track, the spinning disc of a cover-less track and the pulsing dot run on a low-rate shared clock (`Controls/Ticker.cs`, 12 / 4 fps). With software rendering a 60 fps loop costs about 25% of a core, while the low-rate clock costs a few percent. Every looping animation pauses while the window is not in front, like the Electron build's app-idle. Set `RETRO_ANIM=always` to keep them running in the background, for measuring
- libmpv in-process engine: no mpv.exe process, no pipe. The C API's replies and events are turned into the IPC JSON messages, so `MpvEngine` keeps one code path for both. A crash inside mpv now takes the app down with it, where a crashed mpv.exe was restarted on the next Play
- Direct output / bit-perfect (`PlayerViewModel.BitPerfect`, `Services/DeviceReservation.cs`, `Retrograde.Audio/AlsaDirect.cs`): a device named `alsa/hw:CARD=<id>,DEV=<n>` is the whole feature, kept in `AudioDevice` like any other output. The list comes from `/proc/asound` (mpv does not list plain `hw:` devices, but takes the name). With `ao=pipewire,alsa` mpv ignores an `alsa/` device name, so `MpvEngine` sets `ao=alsa` for these (also at runtime, followed by `ao-reload`). The card is taken from PipeWire with `org.freedesktop.ReserveDevice1`: ask the holder (`RequestRelease`, WirePlumber answers true and drops the card), then request the bus name with `ReplaceExisting`, because WirePlumber asks for the name again within a millisecond (a plain request loses that race, and a Tmds connection can ask for a name only once, so each try gets its own connection). The reservation lives as long as the direct device is selected and mpv is started; dropping the connection hands the card back to WirePlumber (the sink is back about 2 s later). While it is active `_bitPerfect` turns the equalizer filters, ReplayGain and crossfade off and keeps mpv's software volume at 100. The volume knob (`Volume`, which the keys, bars, wheel and MPRIS all set) is written to the card's hardware mixer instead (`Retrograde.Audio/AlsaMixer.cs`, libasound: first of Master / PCM / Speaker / Headphone ... with a playback volume; knob 0..1 is linear in dB over the top 60 dB of the control's range, so a USB DAC with a 30 dB range spreads over the whole knob). Writes are merged while dragging; a 1 s poll (window visible only) reads the control back, so the DAC's own buttons and alsamixer move the knob. The knob is set from the card when direct output starts and goes back to the saved software volume when it ends; the hardware level is never saved. PipeWire's volume (system volume, media keys) addresses a sink that does not exist while the card is reserved. The status line compares mpv's `audio-params` / `audio-out-params` with the kernel's `/proc/asound/cardN/pcm*p/sub0/hw_params` and asks mpv for `af`, `volume`, `replaygain` and `speed` before it says BIT-PERFECT. Measured on the built-in card with silent FLACs: 44.1/16 -> S16_LE 44100, 96/24 -> S32_LE 96000, 44.1/24 -> S32_LE 44100.
- Crossfade (`PlayerViewModel.Crossfade`): a second engine starts the next track at volume 0 and the two cross over equal-power curves (mpv's volume is cubic, so the volume set is the cube root of the amplitude), then they trade places. Off in direct output (one stream on the card), for repeat-one, for tracks shorter than twice the fade, and between tracks of one album played in order (they stay gapless)
- Equalizer (`PlayerViewModel.Equalizer`): one labelled lavfi `equalizer` per octave band in mpv's `af` chain plus a preamp (`volume`) that takes off the largest boost. A band change is an `af-command` with the ffmpeg filter as target (sent to `all`, the graph's format filters reject it), so nothing is rebuilt and playback does not stutter
- Shuffle plays the whole library (`PlayerViewModel.Shuffle`): turning it on keeps the current track and puts every other library track behind it in a random round; turning it off goes back to the queue from before at the current track (after a restart, to the track's album). Playing an album with shuffle on starts the chosen track, then the library
- Queue: tracks queued by hand form a line right after the current track (`PlayerViewModel._queuedByHand`): ADD_TO_QUEUE joins its end, PLAY_NEXT its front, and it plays before the rest of the queue, in shuffle too. Playing on in order uses it up; jumping elsewhere ends it (the skipped tracks stay as ordinary queue entries); queue popup rows drag by the grip that replaces the number on hover (in shuffle the play order changes, not the queue)
- Play counts (`Core/PlayStats`, native-stats.json): counted after half the track or four minutes of listening, seeks excluded. Library sorts RECENTLY PLAYED and MOST PLAYED; the grid is not reordered while it is on screen
- Lyrics (`Core/Lyrics`): a `.lrc` next to the track, else the lyrics tag. LRC stamps (several per line, `[offset:]`, enhanced word stamps dropped) make them synced. Shown in a LYRICS tab of the full-screen player; the position is polled 4x a second while synced lyrics show (2x otherwise) and the sung line is scrolled to a third of the height
- Folder watching (`MainViewModel.Watch`): a FileSystemWatcher per library root; 4 s after the last event the new and changed audio files (or moved-in folders) are scanned quietly, files still being copied wait for the next round. Deletions are left to F5, which asks first. Tray menu toggle, saved in native-settings.json

- Smart playlists (`Core/SmartPlaylists`, `MainViewModel.Smart`): rules over title / artist / album text, format, bit depth, sample rate, year, length, plays, last played and date added, plus favorite; all or any must match; ordered (album order, recently added, most / recently played, title, random) and optionally limited. Worked out each time they show, so they follow the library and play counts; the counts in the sidebar are refreshed after a load, a like and a counted play. Stored in native-collections.json under `smart`; the first start makes four examples. "Random" is the same order all day, so a like does not reshuffle the page. A rule left without a value is dropped on save
- Artists (`MainViewModel.Artists`): an artist is an album artist ("Various Artists" included). ALBUMS / ARTISTS switch on the LIBRARY_DATA header (remembered, sorts apply to artists by their newest album, name, or summed plays), an ARTISTS tab in SEARCH, and an artist page with the five most played tracks and the albums, newest first. The artist line of an album page and of the player bar opens it
- Page history: a page opened from a page (album -> artist -> album) goes back one page at a time; the back button reads BACK, then the view the pages came from (LIBRARY / SEARCH)
- Detail pages are virtualized (a ListBox on a VirtualizingStackPanel, like the library grid) and their rows are swapped in with one Reset (`BulkObservableCollection`): an 848-track playlist shows in about 0.25 s and the app stays usable meanwhile, where building every row froze it for seconds. The rows do not clip, so the header's cover colour still fades over the first rows. Only an album page is split into discs; playlists mix albums, so their disc numbers are not grouped any more
- Tag editor (`Core/TagWriter`, `MainViewModel.Tags`): EDIT_TAGS on a track (title, artist, album, album artist, year, track, disc, cover) and EDIT_ALBUM_TAGS on an album page (album, album artist, year, cover for every file). Only changed fields are written. A new or removed picture clears the album's cached cover first; then the files are scanned again, so a renamed album moves (and gets its cover again), the page on screen reopens under the new name and the queue takes the new Track objects without interrupting playback
- Mini player (`Views/MiniPlayerWindow`): 400x122, always on top, not in the taskbar; cover on the left, then title and artist, the transport centred, and a short seek strip between the two times; a title or artist too long for its space scrolls while the pointer is over the window (`Controls/Marquee`: to the end, a rest, back, a rest; trimmed again when the pointer leaves); drag anywhere to move, the place is remembered (bottom right the first time). Ctrl+M or the player bar button, Escape or its expand button to go back; tray "Show Player" and a second launch also go back. The position keeps updating while the main window is hidden
- Taskbar thumbnail buttons (`Services/TaskbarButtons`): ITaskbarList3 called through its vtable (no built-in COM interop, so trimming is safe); icons are the app's glyphs rendered white at the small-icon size; clicks arrive as WM_COMMAND / THBN_CLICKED through `Win32Properties.AddWndProcHookCallback`. Added again on TaskbarButtonCreated (Explorer restart)
- Accent from the cover (`Services/AccentTheme`): the vivid colour of the playing track's cover (CoverColors, lifted to read on #121212), faded over 400 ms into the shared NowAccent / NowAccentSoft brushes used by the playing row, the card title and the sung lyric line; the seek strip draws a gradient of it. A small coloured detail is enough (0.2% of the picture, weighted); a cover with no colour at all gives a silver of its own tone. The setting off keeps the orange. Tray menu toggle, saved as AccentFromCover
- `?` opens and closes the shortcut list (SYSTEM_MANUAL); Escape closes the tag and smart playlist editors from inside their text boxes too

Not yet:
- nothing planned
