using System.ComponentModel;
using System.Runtime.InteropServices;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Retrograde.App.ViewModels;

namespace Retrograde.App.Services;

/// <summary>
/// Previous / play-pause / next buttons in the window's taskbar thumbnail (ITaskbarList3 thumbnail
/// toolbar). The COM object is called through its vtable, so no built-in COM interop is needed (trimmed
/// builds). Icons are the app's own glyphs, drawn white at the small-icon size. Clicks arrive as
/// WM_COMMAND with THBN_CLICKED in the window procedure.
/// </summary>
public static unsafe class TaskbarButtons
{
    private const uint IdPrev = 1, IdPlay = 2, IdNext = 3;
    private const uint WM_COMMAND = 0x0111;
    private const int THBN_CLICKED = 0x1800;
    private const uint THB_ICON = 0x2, THB_TOOLTIP = 0x4, THB_FLAGS = 0x8;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct THUMBBUTTON
    {
        public uint dwMask;
        public uint iId;
        public uint iBitmap;
        public IntPtr hIcon;
        public fixed char szTip[260];
        public uint dwFlags;
    }

    private static readonly Guid ClsidTaskbarList = new("56FDF344-FD6D-11d0-958A-006097C9A090");
    private static readonly Guid IidTaskbarList3 = new("EA1AFB91-9E28-4B86-90E9-9E9F8A5EEFAF");

    private static IntPtr _list;
    private static IntPtr _hwnd;
    private static uint _createdMessage;
    private static bool _added;
    private static PlayerViewModel? _player;
    private static IntPtr _icPrev, _icPlay, _icPause, _icNext;

    [DllImport("ole32.dll")]
    private static extern int CoCreateInstance(in Guid clsid, IntPtr outer, uint context, in Guid iid, out IntPtr obj);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterWindowMessageW(string name);

    [DllImport("user32.dll")]
    private static extern IntPtr CreateIcon(IntPtr instance, int width, int height, byte planes, byte bitsPixel, byte[] andBits, byte[] xorBits);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);

    /// <summary>Hooks the window and adds the buttons once the taskbar has its button for it.</summary>
    public static void Attach(Window window, PlayerViewModel player)
    {
        if (!OperatingSystem.IsWindows() || _player is not null) return;
        try
        {
            _hwnd = window.TryGetPlatformHandle()?.Handle ?? IntPtr.Zero;
            if (_hwnd == IntPtr.Zero) return;
            if (CoCreateInstance(ClsidTaskbarList, IntPtr.Zero, 1 /* CLSCTX_INPROC_SERVER */, IidTaskbarList3, out _list) != 0 || _list == IntPtr.Zero)
                return;
            var vtbl = *(IntPtr**)_list;
            ((delegate* unmanaged[Stdcall]<IntPtr, int>)vtbl[3])(_list); // HrInit
            _player = player;
            var size = Math.Max(16, GetSystemMetrics(49 /* SM_CXSMICON */));
            _icPrev = MakeIcon("IcSkipBack", size);
            _icPlay = MakeIcon("IcPlay", size);
            _icPause = MakeIcon("IcPause", size);
            _icNext = MakeIcon("IcSkipForward", size);
            _createdMessage = RegisterWindowMessageW("TaskbarButtonCreated");
            Win32Properties.AddWndProcHookCallback(window, WndProc);
            player.PropertyChanged += OnPlayerChanged;
            TryAdd(); // the taskbar button usually exists by the time the window has opened
        }
        catch (Exception ex)
        {
            Retrograde.Core.ErrorLog.Write("taskbar buttons", ex);
        }
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == _createdMessage && _createdMessage != 0)
        {
            _added = false; // Explorer restarted, or the button was made after us: add them (again)
            TryAdd();
        }
        else if (msg == WM_COMMAND && ((long)wParam >> 16 & 0xFFFF) == THBN_CLICKED && _player is { } p)
        {
            switch ((uint)((long)wParam & 0xFFFF))
            {
                case IdPrev: p.PrevCommand.Execute(null); break;
                case IdPlay: p.TogglePlayCommand.Execute(null); break;
                case IdNext: p.NextCommand.Execute(null); break;
                default: return IntPtr.Zero;
            }
            handled = true;
        }
        return IntPtr.Zero;
    }

    private static void OnPlayerChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(PlayerViewModel.IsPlaying) or nameof(PlayerViewModel.HasTrack)) Update();
    }

    private static void Fill(ref THUMBBUTTON b, uint id, IntPtr icon, string tip, bool enabled)
    {
        b.dwMask = THB_ICON | THB_TOOLTIP | THB_FLAGS;
        b.iId = id;
        b.hIcon = icon;
        b.dwFlags = enabled ? 0u : 1u; // THBF_ENABLED / THBF_DISABLED
        fixed (char* dst = b.szTip)
        {
            var n = Math.Min(tip.Length, 259);
            for (int i = 0; i < n; i++) dst[i] = tip[i];
            dst[n] = '\0';
        }
    }

    private static int Send(int slot)
    {
        var playing = _player?.IsPlaying == true;
        var hasTrack = _player?.HasTrack == true;
        var buttons = stackalloc THUMBBUTTON[3];
        Fill(ref buttons[0], IdPrev, _icPrev, "Previous", hasTrack);
        Fill(ref buttons[1], IdPlay, playing ? _icPause : _icPlay, playing ? "Pause" : "Play", hasTrack);
        Fill(ref buttons[2], IdNext, _icNext, "Next", hasTrack);
        var vtbl = *(IntPtr**)_list;
        return ((delegate* unmanaged[Stdcall]<IntPtr, IntPtr, uint, THUMBBUTTON*, int>)vtbl[slot])(_list, _hwnd, 3, buttons);
    }

    private static void TryAdd()
    {
        if (_list == IntPtr.Zero || _added) return;
        try
        {
            var hr = Send(15 /* ThumbBarAddButtons */);
            _added = hr == 0;
            if (!_added) Retrograde.Core.ErrorLog.Write("taskbar buttons", new InvalidOperationException($"ThumbBarAddButtons returned 0x{hr:X8}"));
        }
        catch (Exception ex) { Retrograde.Core.ErrorLog.Write("taskbar buttons", ex); }
    }

    private static void Update()
    {
        if (_list == IntPtr.Zero) return;
        if (!_added) { TryAdd(); return; }
        try { Send(16 /* ThumbBarUpdateButtons */); }
        catch (Exception ex) { Retrograde.Core.ErrorLog.Write("taskbar buttons", ex); }
    }

    /// <summary>One of the app's glyphs, filled and stroked white, as a 32-bit icon.</summary>
    private static IntPtr MakeIcon(string key, int size)
    {
        if (Application.Current is not { } app || !app.TryGetResource(key, null, out var res) || res is not Geometry geometry) return IntPtr.Zero;
        using var rtb = new RenderTargetBitmap(new PixelSize(size, size), new Vector(96, 96));
        using (var ctx = rtb.CreateDrawingContext())
        using (ctx.PushTransform(Matrix.CreateScale(size / 24.0, size / 24.0)))
        {
            var pen = new Pen(Brushes.White, 2, lineCap: PenLineCap.Round, lineJoin: PenLineJoin.Round);
            ctx.DrawGeometry(Brushes.White, pen, geometry);
        }
        var stride = size * 4;
        var pixels = new byte[stride * size];
        fixed (byte* p = pixels)
            rtb.CopyPixels(new PixelRect(0, 0, size, size), (IntPtr)p, pixels.Length, stride);
        // Premultiplied BGRA from the renderer; icons take straight alpha
        for (int i = 0; i < pixels.Length; i += 4)
        {
            var a = pixels[i + 3];
            if (a is 0 or 255) continue;
            pixels[i] = (byte)Math.Min(255, pixels[i] * 255 / a);
            pixels[i + 1] = (byte)Math.Min(255, pixels[i + 1] * 255 / a);
            pixels[i + 2] = (byte)Math.Min(255, pixels[i + 2] * 255 / a);
        }
        var maskStride = (size + 15) / 16 * 2; // AND mask rows are WORD aligned; all zero: alpha decides
        return CreateIcon(IntPtr.Zero, size, size, 1, 32, new byte[maskStride * size], pixels);
    }
}
