#:property TargetFramework=net10.0-windows10.0.19041.0
// Lists Windows media sessions (what the volume flyout shows); "next" sends Next to Retrograde's session.
using Windows.Media.Control;

var mgr = await GlobalSystemMediaTransportControlsSessionManager.RequestAsync();
foreach (var s in mgr.GetSessions())
{
    var props = await s.TryGetMediaPropertiesAsync();
    var info = s.GetPlaybackInfo();
    var tl = s.GetTimelineProperties();
    Console.WriteLine($"{s.SourceAppUserModelId} | {props.Title} | {props.Artist} | {props.AlbumTitle} | {info.PlaybackStatus} | {tl.Position:mm\\:ss}/{tl.EndTime:mm\\:ss} | thumb={(props.Thumbnail is null ? "no" : "yes")} | next={info.Controls.IsNextEnabled}");
    if (args.Length > 0 && args[0] == "next" && s.SourceAppUserModelId.Contains("Retrograde", StringComparison.OrdinalIgnoreCase))
        Console.WriteLine($"  next sent: {await s.TrySkipNextAsync()}");
}
