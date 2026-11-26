# Drives the real dotori windows on Windows without touching the owner's screen, mouse or keyboard.
# The window opens on a desktop of its own that is never shown, input reaches it as window messages,
# and pictures come from the window itself. The pointer moves twice with a pause between, since
# GPUI may miss the first move after the pointer left.
#   win.ps1 start <home> [which] [look] [ko|en]   open `dotori <which>` (window by default) with DOTORI_HOME=<home>
#   win.ps1 move <home> <x> <y>             put the pointer at x, y in window pixels before scaling
#   win.ps1 click <home> <x> <y> [frames <out.png>]   click there; with frames, save that many pictures right after, out-1.png on
#   win.ps1 scroll <home> <x> <y> <ticks>   turn the wheel there, down for a positive count
#   win.ps1 type <home> <text>              type text
#   win.ps1 key <home> <enter|tab|escape|backspace>
#   win.ps1 blur <home>                     tell the window it lost focus, as when the owner clicks elsewhere
#   win.ps1 shot <home> <out.png>           save the window
#   win.ps1 stop <home>                     close it
#   [look] is light, dark, a theme such as forest, or both as forest-dark
param([Parameter(Position = 0)][string]$Command, [Parameter(Position = 1)][string]$Home_, [Parameter(Position = 2, ValueFromRemainingArguments)][string[]]$Rest)
$ErrorActionPreference = 'Stop'

Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class Unseen {
    public const string Desktop = "dotori-unseen";

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct StartupInfo {
        public int cb; public string reserved; public string desktop; public string title;
        public int x, y, width, height, xChars, yChars, fill, flags;
        public short show, reserved2; public IntPtr reserved3, stdin, stdout, stderr;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct ProcessInfo { public IntPtr process, thread; public int processId, threadId; }
    [StructLayout(LayoutKind.Sequential)]
    struct Rect { public int Left, Top, Right, Bottom; }
    delegate bool EachWindow(IntPtr window, IntPtr data);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr CreateDesktop(string name, IntPtr device, IntPtr mode, uint flags, uint access, IntPtr security);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr OpenDesktop(string name, uint flags, bool inherit, uint access);
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool SetThreadDesktop(IntPtr desktop);
    [DllImport("user32.dll")]
    static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll")]
    static extern bool EnumDesktopWindows(IntPtr desktop, EachWindow each, IntPtr data);
    [DllImport("user32.dll")]
    static extern int GetWindowThreadProcessId(IntPtr window, out int process);
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr window);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string app, StringBuilder line, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref StartupInfo start, out ProcessInfo info);
    [DllImport("user32.dll")]
    static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll")]
    static extern bool GetClientRect(IntPtr window, out Rect rect);
    [StructLayout(LayoutKind.Sequential)]
    struct Spot { public int X, Y; }
    [DllImport("user32.dll")]
    static extern bool ClientToScreen(IntPtr window, ref Spot spot);
    [DllImport("user32.dll")]
    static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")]
    static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")]
    static extern bool SetProcessDPIAware();

    const uint AllAccess = 0x01FF;

    static T OnDesktop<T>(Func<T> work) {
        T result = default(T);
        Exception failure = null;
        var thread = new Thread(() => {
            try {
                IntPtr desktop = OpenDesktop(Desktop, 0, false, AllAccess);
                if (desktop == IntPtr.Zero) throw new Exception("no unseen desktop; run start first");
                try {
                    if (!SetThreadDesktop(desktop)) throw new Exception("could not join the unseen desktop: " + Marshal.GetLastWin32Error());
                    result = work();
                } finally { CloseDesktop(desktop); }
            } catch (Exception e) { failure = e; }
        });
        thread.Start();
        thread.Join();
        if (failure != null) throw failure;
        return result;
    }

    public static int Start(string line) {
        SetProcessDPIAware();
        IntPtr desktop = CreateDesktop(Desktop, IntPtr.Zero, IntPtr.Zero, 0, AllAccess, IntPtr.Zero);
        if (desktop == IntPtr.Zero) throw new Exception("could not make the unseen desktop: " + Marshal.GetLastWin32Error());
        var start = new StartupInfo();
        start.cb = Marshal.SizeOf(start);
        start.desktop = "WinSta0\\" + Desktop;
        ProcessInfo info;
        bool started = CreateProcess(null, new StringBuilder(line), IntPtr.Zero, IntPtr.Zero, false, 0x08000000, IntPtr.Zero, null, ref start, out info);
        int failure = Marshal.GetLastWin32Error();
        CloseDesktop(desktop);
        if (!started) throw new Exception("could not start dotori: " + failure);
        return info.processId;
    }

    public static IntPtr Find(int process) {
        return OnDesktop(() => {
            IntPtr found = IntPtr.Zero;
            int largest = 100 * 100;
            IntPtr desktop = OpenDesktop(Desktop, 0, false, AllAccess);
            try {
            EnumDesktopWindows(desktop, (window, data) => {
                int owner;
                GetWindowThreadProcessId(window, out owner);
                Rect rect;
                if (owner != process || !IsWindowVisible(window) || !GetClientRect(window, out rect)) return true;
                int area = (rect.Right - rect.Left) * (rect.Bottom - rect.Top);
                if (area > largest) { largest = area; found = window; }
                return true;
            }, IntPtr.Zero);
            } finally { CloseDesktop(desktop); }
            return found;
        });
    }

    public static double Scale(IntPtr window) {
        SetProcessDPIAware();
        uint dpi = GetDpiForWindow(window);
        return dpi == 0 ? 1.0 : dpi / 96.0;
    }

    static IntPtr Point(int x, int y) { return (IntPtr)(((long)(Math.Max(y, 0) & 0xFFFF) << 16) | (long)(Math.Max(x, 0) & 0xFFFF)); }

    public static void Post(IntPtr window, uint message, long w, int x, int y) {
        OnDesktop(() => PostMessage(window, message, (IntPtr)w, Point(x, y)));
    }

    public static void Wheel(IntPtr window, int x, int y, int ticks) {
        var spot = new Spot { X = x, Y = y };
        ClientToScreen(window, ref spot);
        long delta = (-120 * ticks) & 0xFFFF;
        Post(window, 0x020A, delta << 16, spot.X, spot.Y);
    }

    public static void PostRaw(IntPtr window, uint message, long w, long l) {
        OnDesktop(() => PostMessage(window, message, (IntPtr)w, (IntPtr)l));
    }

    public static void Save(IntPtr window, string path) {
        SetProcessDPIAware();
        OnDesktop(() => {
            Rect rect;
            GetClientRect(window, out rect);
            using (var picture = new Bitmap(rect.Right - rect.Left, rect.Bottom - rect.Top, PixelFormat.Format32bppArgb))
            using (var canvas = Graphics.FromImage(picture)) {
                IntPtr dc = canvas.GetHdc();
                bool drawn = PrintWindow(window, dc, 3);
                canvas.ReleaseHdc(dc);
                if (!drawn) throw new Exception("the window did not draw itself");
                picture.Save(path, ImageFormat.Png);
            }
            return true;
        });
    }
}
"@

function Usage { Get-Content $PSCommandPath -TotalCount 14 | Select-Object -Skip 4 | Write-Host; exit 2 }
if (-not $Command -or -not $Home_) { Usage }
New-Item -ItemType Directory -Force $Home_ | Out-Null
$homeDir = (Resolve-Path $Home_).Path
$state = Join-Path $homeDir 'unseen.json'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$dotori = Join-Path $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }) 'debug\dotori.exe'

function Window {
    if (-not (Test-Path $state)) { throw "nothing open; run start first" }
    $open = Get-Content $state | ConvertFrom-Json
    [IntPtr][long]$open.window
}

function At([IntPtr]$window, [string]$x, [string]$y) {
    $scale = [Unseen]::Scale($window)
    @([int]([double]$x * $scale), [int]([double]$y * $scale))
}

switch ($Command) {
    'start' {
        $which = if ($Rest.Count -ge 1) { $Rest[0] } else { 'window' }
        $look = if ($Rest.Count -ge 2) { $Rest[1] } else { 'light' }
        $language = if ($Rest.Count -ge 3) { $Rest[2] } else { $null }
        if (Test-Path $state) { & $PSCommandPath stop $homeDir }
        $kept = @{}
        foreach ($name in 'DOTORI_HOME', 'DOTORI_THEME', 'DOTORI_LANGUAGE') { $kept[$name] = [Environment]::GetEnvironmentVariable($name) }
        try {
            [Environment]::SetEnvironmentVariable('DOTORI_HOME', $homeDir)
            [Environment]::SetEnvironmentVariable('DOTORI_THEME', $look)
            [Environment]::SetEnvironmentVariable('DOTORI_LANGUAGE', $language)
            $process = [Unseen]::Start("`"$dotori`" $which")
        } finally {
            foreach ($name in $kept.Keys) { [Environment]::SetEnvironmentVariable($name, $kept[$name]) }
        }
        $window = [IntPtr]::Zero
        for ($i = 0; $i -lt 100 -and $window -eq [IntPtr]::Zero; $i++) {
            Start-Sleep -Milliseconds 100
            $window = [Unseen]::Find($process)
        }
        if ($window -eq [IntPtr]::Zero) { throw "no $which window" }
        @{ process = $process; window = [long]$window } | ConvertTo-Json | Set-Content $state
        Start-Sleep -Seconds 2
        "ready: $which"
    }
    'move' {
        $window = Window; $p = At $window $Rest[0] $Rest[1]
        [Unseen]::Post($window, 0x0200, 0, $p[0] - 1, $p[1] - 1)
        Start-Sleep -Milliseconds 200
        [Unseen]::Post($window, 0x0200, 0, $p[0], $p[1])
        Start-Sleep -Milliseconds 400
    }
    'click' {
        $window = Window; $p = At $window $Rest[0] $Rest[1]
        [Unseen]::Post($window, 0x0200, 0, $p[0], $p[1])
        [Unseen]::Post($window, 0x0201, 1, $p[0], $p[1])
        [Unseen]::Post($window, 0x0202, 0, $p[0], $p[1])
        if ($Rest.Count -ge 4) {
            $out = [IO.Path]::GetFullPath($Rest[3])
            for ($n = 1; $n -le [int]$Rest[2]; $n++) {
                [Unseen]::Save($window, ($out -replace '\.png$', "-$n.png"))
                Start-Sleep -Milliseconds 30
            }
        }
        Start-Sleep -Milliseconds 500
    }
    'scroll' {
        $window = Window; $p = At $window $Rest[0] $Rest[1]
        [Unseen]::Wheel($window, $p[0], $p[1], [int]$Rest[2])
        Start-Sleep -Milliseconds 400
    }
    'type' {
        $window = Window
        foreach ($char in ($Rest -join ' ').ToCharArray()) { [Unseen]::PostRaw($window, 0x0102, [int]$char, 1) }
        Start-Sleep -Milliseconds 300
    }
    'key' {
        $window = Window
        $code = @{ enter = 0x0D; tab = 0x09; escape = 0x1B; backspace = 0x08 }[$Rest[0]]
        if (-not $code) { Usage }
        [Unseen]::PostRaw($window, 0x0100, $code, 1)
        [Unseen]::PostRaw($window, 0x0101, $code, 0xC0000001)
        Start-Sleep -Milliseconds 300
    }
    'blur' {
        $window = Window
        [Unseen]::PostRaw($window, 0x0006, 0, 0)
        Start-Sleep -Milliseconds 400
    }
    'shot' {
        $window = Window
        $out = [IO.Path]::GetFullPath($Rest[0])
        [Unseen]::Save($window, $out)
        "saved: $out"
    }
    'stop' {
        if (Test-Path $state) {
            $open = Get-Content $state | ConvertFrom-Json
            Stop-Process -Id $open.process -Force -ErrorAction SilentlyContinue
            Remove-Item $state
        }
    }
    default { Usage }
}
