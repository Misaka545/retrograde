#:project ../src/Retrograde.Core/Retrograde.Core.csproj
#:project ../src/Retrograde.Audio/Retrograde.Audio.csproj
// Plays the first library track muted for a few seconds and reports engine events.
using Retrograde.Audio;
using Retrograde.Core;
var albums = await LibraryStore.LoadAsync();
var track = albums.First(a => a.Tracks.Count > 3).Tracks[0];
int tp = 0, durN = 0; double lastT = 0, dur = 0; bool? paused = null;
var engine = new MpvEngine(Path.GetFullPath("../bin/mpv.exe"));
engine.TimePosChanged += t => { tp++; lastT = t; };
engine.DurationChanged += d => { durN++; dur = d; };
engine.PauseChanged += p => paused = p;
var sw = System.Diagnostics.Stopwatch.StartNew();
await engine.StartAsync(exclusive: false);
Console.WriteLine($"start: {sw.ElapsedMilliseconds}ms");
await engine.SetVolumeAsync(0);
await engine.PlayAsync(track.FilePath);
await Task.Delay(3000);
Console.WriteLine($"after 3s: timePosEvents={tp} pos={lastT:F2} duration={dur:F1} paused={paused}");
await engine.SeekAsync(60); await Task.Delay(800);
Console.WriteLine($"after seek 60: pos={lastT:F2}");
await engine.SetPausedAsync(true); await Task.Delay(500);
var p1 = lastT; await Task.Delay(1000);
Console.WriteLine($"paused={paused} posStable={Math.Abs(lastT - p1) < 0.01}");
await engine.PlayAsync(track.FilePath, 30); await Task.Delay(1500);
Console.WriteLine($"replay at 30s: pos={lastT:F2} paused={paused}");
await engine.DisposeAsync();
await Task.Delay(500);
Console.WriteLine($"mpv left running: {System.Diagnostics.Process.GetProcessesByName("mpv").Length}");
