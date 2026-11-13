using Avalonia;
using Avalonia.Controls;
using Avalonia.Data.Converters;
using Avalonia.Media;

namespace Retrograde.App.Controls;

public static class UiConverters
{
    private static readonly IBrush Orange = new SolidColorBrush(Color.Parse("#FF6B35"));
    private static readonly IBrush Gold = new SolidColorBrush(Color.Parse("#E8C060"));

    /// <summary>Search section headers: flush for the first section, 40px gap (24 row gap + 16) before later ones.</summary>
    public static readonly IValueConverter SectionMargin =
        new FuncValueConverter<bool, Thickness>(first => first ? new Thickness(0, 0, 0, 16) : new Thickness(0, 16, 0, 16));

    /// <summary>Icon resource key (e.g. "IcHeart") → geometry from App.axaml.</summary>
    public static readonly IValueConverter IconGeometry =
        new FuncValueConverter<string?, Geometry?>(key =>
            key is not null && Application.Current?.TryFindResource(key, out var g) == true ? g as Geometry : null);

    /// <summary>Playlist bullet in menus: orange when the track is already in the playlist, gold otherwise.</summary>
    public static readonly IValueConverter CheckedBullet =
        new FuncValueConverter<bool, IBrush>(isIn => isIn ? Orange : Gold);

    /// <summary>Hides content but keeps its layout space (1 → shown, 0 → transparent).</summary>
    public static readonly IValueConverter VisibleOpacity =
        new FuncValueConverter<bool, double>(visible => visible ? 1.0 : 0.0);
}
