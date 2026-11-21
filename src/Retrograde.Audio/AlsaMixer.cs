using System.Runtime.InteropServices;

namespace Retrograde.Audio;

/// <summary>
/// The sound card's own hardware volume (its ALSA mixer control, e.g. "PCM" on a USB DAC). Direct output
/// bypasses PipeWire, so its volume (and the system volume that sets it) has no effect there; this is the
/// volume that does, and it leaves the samples the player sends untouched. A control is opened for every
/// call and closed again, so a read always sees what the card says now (a knob on the DAC, alsamixer ...).
/// </summary>
public static class AlsaMixer
{
    private const string Lib = "libasound.so.2";

    /// <summary>Controls that set a card's playback volume, in the order they are tried.</summary>
    private static readonly string[] Candidates = ["Master", "PCM", "Speaker", "Headphone", "Digital", "Line Out", "Front"];

    /// <summary>The 0..1 range of the volume knob maps onto the top this many dB of the control (0.01 dB units).</summary>
    private const long MaxSpanCentiDb = 6000;

    /// <summary>The name of the control that sets the card's playback volume; null when it has none.</summary>
    public static string? FindControl(int card)
    {
        foreach (var name in Candidates)
            if (With(card, name, _ => true)) return name;
        return null;
    }

    /// <summary>The volume of the control as a 0..1 knob position (linear in dB over the top 60 dB of its range); null if it cannot be read.</summary>
    public static double? ReadLevel(int card, string control)
    {
        double? level = null;
        With(card, control, elem =>
        {
            if (snd_mixer_selem_get_playback_dB_range(elem, out var min, out var max) == 0 && max > min
                && snd_mixer_selem_get_playback_dB(elem, 0, out var db) == 0)
            {
                var lo = Math.Max(min, max - MaxSpanCentiDb);
                level = Math.Clamp((double)(db - lo) / (max - lo), 0, 1);
                return true;
            }
            if (snd_mixer_selem_get_playback_volume_range(elem, out var rmin, out var rmax) == 0 && rmax > rmin
                && snd_mixer_selem_get_playback_volume(elem, 0, out var raw) == 0)
            {
                level = Math.Clamp((double)(raw - rmin) / (rmax - rmin), 0, 1);
                return true;
            }
            return false;
        });
        return level;
    }

    /// <summary>Sets the control to a 0..1 knob position; 0 is the control's lowest setting. False if the card refused.</summary>
    public static bool WriteLevel(int card, string control, double level)
    {
        level = Math.Clamp(level, 0, 1);
        return With(card, control, elem =>
        {
            // A control that has an on/off switch is switched on: a muted card would ignore the knob
            if (snd_mixer_selem_has_playback_switch(elem) != 0) snd_mixer_selem_set_playback_switch_all(elem, 1);
            if (snd_mixer_selem_get_playback_dB_range(elem, out var min, out var max) == 0 && max > min)
            {
                var lo = Math.Max(min, max - MaxSpanCentiDb);
                var db = level <= 0 ? min : lo + (long)Math.Round((max - lo) * level);
                return snd_mixer_selem_set_playback_dB_all(elem, db, 0) >= 0;
            }
            if (snd_mixer_selem_get_playback_volume_range(elem, out var rmin, out var rmax) == 0 && rmax > rmin)
                return snd_mixer_selem_set_playback_volume_all(elem, rmin + (long)Math.Round((rmax - rmin) * level)) >= 0;
            return false;
        });
    }

    /// <summary>Opens the card's mixer, finds the playback-volume control by name and runs <paramref name="action"/> on it.</summary>
    private static bool With(int card, string control, Func<IntPtr, bool> action)
    {
        IntPtr mixer = IntPtr.Zero, id = IntPtr.Zero;
        try
        {
            if (snd_mixer_open(out mixer, 0) < 0) return false;
            if (snd_mixer_attach(mixer, $"hw:{card}") < 0) return false;
            if (snd_mixer_selem_register(mixer, IntPtr.Zero, IntPtr.Zero) < 0) return false;
            if (snd_mixer_load(mixer) < 0) return false;
            if (snd_mixer_selem_id_malloc(out id) < 0) return false;
            snd_mixer_selem_id_set_index(id, 0);
            snd_mixer_selem_id_set_name(id, control);
            var elem = snd_mixer_find_selem(mixer, id);
            if (elem == IntPtr.Zero || snd_mixer_selem_has_playback_volume(elem) == 0) return false;
            return action(elem);
        }
        catch (Exception ex) when (ex is DllNotFoundException or EntryPointNotFoundException)
        {
            return false; // no libasound: no hardware volume
        }
        finally
        {
            if (id != IntPtr.Zero) snd_mixer_selem_id_free(id);
            if (mixer != IntPtr.Zero) snd_mixer_close(mixer);
        }
    }

    [DllImport(Lib)] private static extern int snd_mixer_open(out IntPtr mixer, int mode);
    [DllImport(Lib)] private static extern int snd_mixer_close(IntPtr mixer);
    [DllImport(Lib)] private static extern int snd_mixer_selem_register(IntPtr mixer, IntPtr options, IntPtr classp);
    [DllImport(Lib)] private static extern int snd_mixer_load(IntPtr mixer);
    [DllImport(Lib)] private static extern int snd_mixer_selem_id_malloc(out IntPtr id);
    [DllImport(Lib)] private static extern void snd_mixer_selem_id_free(IntPtr id);
    [DllImport(Lib)] private static extern void snd_mixer_selem_id_set_index(IntPtr id, uint index);
    [DllImport(Lib)] private static extern IntPtr snd_mixer_find_selem(IntPtr mixer, IntPtr id);
    [DllImport(Lib)] private static extern int snd_mixer_selem_has_playback_volume(IntPtr elem);
    [DllImport(Lib)] private static extern int snd_mixer_selem_has_playback_switch(IntPtr elem);
    [DllImport(Lib)] private static extern int snd_mixer_selem_set_playback_switch_all(IntPtr elem, int value);
    [DllImport(Lib)] private static extern int snd_mixer_selem_get_playback_dB_range(IntPtr elem, out long min, out long max);
    [DllImport(Lib)] private static extern int snd_mixer_selem_get_playback_dB(IntPtr elem, int channel, out long value);
    [DllImport(Lib)] private static extern int snd_mixer_selem_set_playback_dB_all(IntPtr elem, long value, int dir);
    [DllImport(Lib)] private static extern int snd_mixer_selem_get_playback_volume_range(IntPtr elem, out long min, out long max);
    [DllImport(Lib)] private static extern int snd_mixer_selem_get_playback_volume(IntPtr elem, int channel, out long value);
    [DllImport(Lib)] private static extern int snd_mixer_selem_set_playback_volume_all(IntPtr elem, long value);

    [DllImport(Lib)]
    private static extern int snd_mixer_attach(IntPtr mixer, [MarshalAs(UnmanagedType.LPUTF8Str)] string name);

    [DllImport(Lib)]
    private static extern void snd_mixer_selem_id_set_name(IntPtr id, [MarshalAs(UnmanagedType.LPUTF8Str)] string name);
}
