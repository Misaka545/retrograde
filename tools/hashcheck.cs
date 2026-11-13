#:project ../src/Retrograde.Core/Retrograde.Core.csproj
using Retrograde.Core;
var albums = await LibraryStore.LoadAsync();
int ok = 0, bad = 0, noCover = 0;
foreach (var a in albums)
{
    if (a.CoverThumbPath is null) { noCover++; continue; }
    var expected = Path.GetFileName(a.CoverThumbPath);
    if (!expected.EndsWith("_thumb.jpg")) continue;
    if (Path.GetFileName(CoverPaths.ThumbPath(a.Name)) == expected) ok++; else { bad++; if (bad < 4) Console.WriteLine($"MISMATCH {a.Name} -> {CoverPaths.HashName(a.Name)} vs {expected}"); }
}
Console.WriteLine($"albums={albums.Count} tracks={albums.Sum(a => a.Tracks.Count)} hashOk={ok} hashBad={bad} noCover={noCover}");
