#:project ../src/Retrograde.Core/Retrograde.Core.csproj
// Scans the given folders into RETRO_APPDIR (required, so the real ~/.retrograde is never touched)
// and prints the resulting albums.
using System.Diagnostics;
using Retrograde.Core;

if (string.IsNullOrEmpty(Environment.GetEnvironmentVariable("RETRO_APPDIR")))
{
    Console.WriteLine("set RETRO_APPDIR to a scratch folder first");
    return;
}
var sw = Stopwatch.StartNew();
var progress = new Progress<ScanProgress>(p => { if (p.Scanned == p.Total) Console.WriteLine($"  {p.Phase} {p.Scanned}/{p.Total}"); });
var r = await Scanner.ScanAsync(args, progress);
Console.WriteLine($"added {r.AddedTracks} tracks, {r.NewAlbums} new albums, {r.Covers} covers in {sw.ElapsedMilliseconds} ms");
foreach (var a in await LibraryStore.LoadAsync())
{
    Console.WriteLine($"{a.Name} | {a.Artist} | {a.Year} | thumb={(a.CoverThumbPath is { } cp && File.Exists(cp) ? new FileInfo(cp).Length + "B" : "none")}");
    foreach (var t in a.Tracks.Take(3)) Console.WriteLine($"   {t.DiscNumber}.{t.TrackNumber} {t.Title} [{t.QualityLabel}] {t.Duration:0.0}s");
}
