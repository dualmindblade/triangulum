<#
.SYNOPSIS
  Drive the game's real window from PowerShell (Windows): start the game,
  click, drag, type, press keys, take screenshots, stop it.

.DESCRIPTION
  For checking things the play harness can't reach: the look panel (F1), the
  block and texture editor, file dialogs, the hotbar as drawn in the app.
  Written for M3b from M3a's notes (docs/missions/M3a-findings.md).

  Dot-source it, then call the functions:

    . game\tools\window.ps1
    $g = Start-Game -Arguments '--save saves/_live --blocks out/live/blocks.json --size 1600x900'
    Wait-GameWindow $g                      # the window titled "Triangulum..." of that process
    Send-GameKey $g F1                      # a key by name (System.Windows.Forms.Keys)
    Click-Game $g 1200 300                  # client pixels, left button
    Click-Game $g 1200 300 -Right
    Drag-Game $g 400 300 520 300            # press, move in steps, release
    Send-GameText $g 'red brick'            # typed text (SendKeys; {ENTER} etc. work)
    Save-GameShot $g out\live\01.png        # the client area as it is on screen
    Stop-Game $g                            # sends Q (save and quit), then waits; kills only if stuck

  Coordinates are client-area pixels (what Save-GameShot writes), so read
  a position off a screenshot and click it. The process is made DPI aware
  first, so pixels are real pixels at any display scaling (M3a: the window
  reported DPI 144 at 150%). The window is found by process id, never by
  title alone (the game is a console program: MainWindowHandle is the
  console). The game window must stay in front while screenshots are taken
  (they copy the screen); Focus-Game brings it back.

  Stop-Game sends Q and waits; it kills the process only if it is still
  there after the timeout, and only the process Start-Game started.
#>

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

if (-not ('TriWin' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class TriWin {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    public static IntPtr Find(uint pid, string prefix) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid || !IsWindowVisible(h)) return true;
            var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
            if (sb.ToString().StartsWith(prefix)) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
}
[void][TriWin]::SetProcessDPIAware()

$script:GameExe = Join-Path $PSScriptRoot '..\target\release\triangulum.exe'

function Start-Game {
    param([string]$Arguments = '', [string]$Exe = $script:GameExe, [string]$Log = '')
    $wd = Split-Path -Parent (Split-Path -Parent $Exe)   # target\release -> target
    $wd = Split-Path -Parent $wd                          # -> game
    $p = @{ FilePath = (Resolve-Path $Exe).Path; ArgumentList = $Arguments; WorkingDirectory = $wd; PassThru = $true }
    if ($Log) { $p.RedirectStandardOutput = $Log; $p.RedirectStandardError = "$Log.err"; $p.WindowStyle = 'Hidden' }
    $proc = Start-Process @p
    [pscustomobject]@{ Process = $proc; Hwnd = [IntPtr]::Zero }
}

function Wait-GameWindow {
    param($Game, [int]$TimeoutSec = 120, [string]$Title = 'Triangulum')
    $t = [Diagnostics.Stopwatch]::StartNew()
    while ($t.Elapsed.TotalSeconds -lt $TimeoutSec) {
        if ($Game.Process.HasExited) { throw "the game exited (code $($Game.Process.ExitCode))" }
        $h = [TriWin]::Find([uint32]$Game.Process.Id, $Title)
        if ($h -ne [IntPtr]::Zero) { $Game.Hwnd = $h; Focus-Game $Game; return $h }
        Start-Sleep -Milliseconds 250
    }
    throw "no window titled $Title from process $($Game.Process.Id) after $TimeoutSec s"
}

function Focus-Game {
    param($Game)
    [void][TriWin]::ShowWindow($Game.Hwnd, 9)   # SW_RESTORE
    [void][TriWin]::BringWindowToTop($Game.Hwnd)
    # Windows refuses SetForegroundWindow unless the caller had the last
    # input; an Alt tap first is the usual way round it.
    [TriWin]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [TriWin]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
    [void][TriWin]::SetForegroundWindow($Game.Hwnd)
    Start-Sleep -Milliseconds 150
}

function Get-GameOrigin {
    param($Game)
    $pt = New-Object TriWin+POINT
    [void][TriWin]::ClientToScreen($Game.Hwnd, [ref]$pt)
    $r = New-Object TriWin+RECT
    [void][TriWin]::GetClientRect($Game.Hwnd, [ref]$r)
    [pscustomobject]@{ X = $pt.X; Y = $pt.Y; Width = $r.Right; Height = $r.Bottom }
}

function Move-GameMouse {
    param($Game, [int]$X, [int]$Y)
    $o = Get-GameOrigin $Game
    [void][TriWin]::SetCursorPos($o.X + $X, $o.Y + $Y)
}

function Click-Game {
    param($Game, [int]$X, [int]$Y, [switch]$Right, [int]$HoldMs = 60)
    Move-GameMouse $Game $X $Y
    Start-Sleep -Milliseconds 60
    $down, $up = if ($Right) { 0x0008, 0x0010 } else { 0x0002, 0x0004 }
    [TriWin]::mouse_event($down, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds $HoldMs
    [TriWin]::mouse_event($up, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}

function Drag-Game {
    param($Game, [int]$X1, [int]$Y1, [int]$X2, [int]$Y2, [int]$Steps = 12, [int]$StepMs = 25)
    Move-GameMouse $Game $X1 $Y1
    Start-Sleep -Milliseconds 60
    [TriWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    for ($i = 1; $i -le $Steps; $i++) {
        Move-GameMouse $Game ([int]($X1 + ($X2 - $X1) * $i / $Steps)) ([int]($Y1 + ($Y2 - $Y1) * $i / $Steps))
        Start-Sleep -Milliseconds $StepMs
    }
    [TriWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}

function Send-GameKey {
    # A key by its System.Windows.Forms.Keys name: F1, F10, Escape, D1, Q, W ...
    param($Game, [string]$Key, [int]$HoldMs = 50)
    $vk = [byte][System.Windows.Forms.Keys]::$Key
    [TriWin]::keybd_event($vk, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds $HoldMs
    [TriWin]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}

function Send-GameText {
    # Typed text into whatever has the keyboard (an egui text field).
    param($Game, [string]$Text)
    [System.Windows.Forms.SendKeys]::SendWait($Text)
    Start-Sleep -Milliseconds 150
}

function Save-GameShot {
    # The client area as it is on screen now (the window must be in front).
    param($Game, [string]$Path)
    $o = Get-GameOrigin $Game
    $bmp = New-Object System.Drawing.Bitmap $o.Width, $o.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($o.X, $o.Y, 0, 0, $bmp.Size)
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    $Path
}

function Stop-Game {
    # Q saves and quits; the process is killed only if it is still there
    # after the timeout (and only the one Start-Game started).
    param($Game, [int]$TimeoutSec = 30)
    if (-not $Game.Process.HasExited) {
        Focus-Game $Game
        Send-GameKey $Game Escape
        Send-GameKey $Game Q
        if (-not $Game.Process.WaitForExit($TimeoutSec * 1000)) {
            Write-Warning "the game did not quit after Q; stopping process $($Game.Process.Id)"
            Stop-Process -Id $Game.Process.Id -Force
        }
    }
    $Game.Process.ExitCode
}
