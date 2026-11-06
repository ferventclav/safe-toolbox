using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Windows.Graphics;

namespace SafeToolbox;

/// <summary>Initial size, centring and a DPI-aware minimum size (WM_GETMINMAXINFO) for the main window.</summary>
internal static class WindowSizing
{
    private const uint WmGetMinMaxInfo = 0x0024;

    private delegate IntPtr SubclassProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr idSubclass, UIntPtr refData);

    [DllImport("comctl32.dll", SetLastError = true)]
    private static extern bool SetWindowSubclass(IntPtr hWnd, SubclassProc proc, UIntPtr idSubclass, UIntPtr refData);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hWnd);

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT
    {
        public int X;
        public int Y;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MINMAXINFO
    {
        public POINT ptReserved;
        public POINT ptMaxSize;
        public POINT ptMaxPosition;
        public POINT ptMinTrackSize;
        public POINT ptMaxTrackSize;
    }

    // Kept in a field so the delegate is not collected while Windows still calls it.
    private static SubclassProc? s_proc;

    public static double Scale(IntPtr hwnd)
    {
        uint dpi = GetDpiForWindow(hwnd);
        return dpi == 0 ? 1.0 : dpi / 96.0;
    }

    /// <summary>Stops the window from being resized below the given size in effective (96 DPI) pixels.</summary>
    public static void SetMinimumSize(IntPtr hwnd, int widthDip, int heightDip)
    {
        s_proc = (hWnd, msg, wParam, lParam, id, refData) =>
        {
            IntPtr result = DefSubclassProc(hWnd, msg, wParam, lParam);
            if (msg == WmGetMinMaxInfo && lParam != IntPtr.Zero)
            {
                double scale = Scale(hWnd);
                MINMAXINFO info = Marshal.PtrToStructure<MINMAXINFO>(lParam);
                info.ptMinTrackSize.X = (int)Math.Round(widthDip * scale);
                info.ptMinTrackSize.Y = (int)Math.Round(heightDip * scale);
                Marshal.StructureToPtr(info, lParam, false);
            }
            return result;
        };
        SetWindowSubclass(hwnd, s_proc, (UIntPtr)1, UIntPtr.Zero);
    }

    /// <summary>
    /// Puts the window back where it was last closed, when that spot is still on a connected screen (a monitor
    /// may have been unplugged since). The window is pulled fully inside that screen's work area, so it never
    /// opens past an edge. Returns false when there is nothing usable to restore.
    /// </summary>
    public static bool Restore(AppWindow window, int[]? bounds)
    {
        if (bounds is not { Length: 4 } || bounds[2] < 200 || bounds[3] < 200)
        {
            return false;
        }
        var saved = new RectInt32(bounds[0], bounds[1], bounds[2], bounds[3]);
        // the title bar's middle has to be on a screen, or the window could not be dragged back
        int gripX = saved.X + saved.Width / 2;
        int gripY = saved.Y + 16;
        // DisplayArea.FindAll() can't be enumerated with foreach (the projection has no IEnumerable behind it),
        // so it is read by index.
        IReadOnlyList<DisplayArea> areas = DisplayArea.FindAll();
        for (int i = 0; i < areas.Count; i++)
        {
            RectInt32 work = areas[i].WorkArea;
            if (gripX >= work.X && gripX < work.X + work.Width && gripY >= work.Y && gripY < work.Y + work.Height)
            {
                RectInt32 rect = FitInside(saved, work);
                // The window starts on the main screen. Moving it to a screen with other scaling makes Windows
                // rescale it on arrival, so the first call gets it there and the second sets the real size.
                window.MoveAndResize(rect);
                window.MoveAndResize(rect);
                return true;
            }
        }
        return false;
    }

    /// <summary>Shrinks the rectangle to fit the work area if needed, then slides it fully inside.</summary>
    public static RectInt32 FitInside(RectInt32 r, RectInt32 work)
    {
        int w = Math.Min(r.Width, work.Width);
        int h = Math.Min(r.Height, work.Height);
        int x = Math.Clamp(r.X, work.X, work.X + work.Width - w);
        int y = Math.Clamp(r.Y, work.Y, work.Y + work.Height - h);
        return new RectInt32(x, y, w, h);
    }

    /// <summary>Sizes the window to the given effective size (shrunk to fit the screen) and centres it.</summary>
    public static void ResizeAndCenter(AppWindow window, IntPtr hwnd, int widthDip, int heightDip)
    {
        double scale = Scale(hwnd);
        int w = (int)Math.Round(widthDip * scale);
        int h = (int)Math.Round(heightDip * scale);
        DisplayArea? area = DisplayArea.GetFromWindowId(window.Id, DisplayAreaFallback.Primary);
        if (area is not null)
        {
            RectInt32 work = area.WorkArea;
            w = Math.Min(w, (int)(work.Width * 0.95));
            h = Math.Min(h, (int)(work.Height * 0.95));
            window.MoveAndResize(new RectInt32(work.X + (work.Width - w) / 2, work.Y + (work.Height - h) / 2, w, h));
        }
        else
        {
            window.Resize(new SizeInt32(w, h));
        }
    }
}
