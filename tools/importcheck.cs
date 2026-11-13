#:project ../src/Retrograde.Core/Retrograde.Core.csproj
// Dry run of the Electron localStorage import: prints what would be imported, writes nothing.
using Retrograde.Core;

var data = ElectronImport.TryRead();
if (data is null) { Console.WriteLine("nothing found"); return; }
Console.WriteLine($"source: {data.ImportedFrom}");
Console.WriteLine($"liked: {data.Liked.Count}");
foreach (var id in data.Liked.Take(3)) Console.WriteLine($"  {id}");
Console.WriteLine($"playlists: {data.Playlists.Count}");
foreach (var p in data.Playlists) Console.WriteLine($"  [{p.Id}] {p.Name}: {p.TrackIds.Count} tracks, cover={p.CoverArt ?? "-"}");
