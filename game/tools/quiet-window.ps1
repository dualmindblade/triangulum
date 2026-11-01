# quiet-window.ps1: test the game's real window while somebody else uses the
# machine. Nothing here sends input or takes focus: the game starts with
# --no-focus (inactive, at the bottom of the z-order), and every call below is
# a window call that names the game's own window. window.ps1 is the other tool:
# it clicks and types for real, so it is only for when the machine is yours.
#
#   . game\tools\quiet-window.ps1
#   $g = Start-QuietGame -Exe E:\triangulum-build\C1\release\triangulum.exe `
#        -Arguments '--size 640x360 --save saves/_quiet --no-sim' -Log out\quiet\run1
#   Hide-Game $g                # minimise, not activated (the taskbar button)
#   Show-Game $g                # restore, not activated
#   Set-GameSize $g 120 1       # as small as Windows allows
#   Send-GameFocus $g $true     # tell the window it has focus (a posted message)
#   Send-GameFocus $g $false    # tell it it lost focus
#   Get-GameCpu $g              # seconds of CPU so far
#   Assert-Quiet $g             # the foreground window and the mouse clip are untouched
#   Stop-QuietGame $g           # WM_CLOSE, the way the X does; kills only if stuck
#
# Output goes to <Log>.out.txt and <Log>.err.txt. Run Get-IdleSeconds first and
# say what it read. Set RUST_BACKTRACE=1 yourself if you want a backtrace on the
# console; a panic in a window also goes to game/out/crash/.

Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public struct QuietRect { public int Left, Top, Right, Bottom; }
public struct QuietLastInput { public uint Size; public uint Time; }
public class QuietWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
  [DllImport("user32.dll", EntryPoint="SetWindowLongPtrW")] public static extern IntPtr SetWindowLongPtr(IntPtr h, int i, IntPtr v);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out QuietRect r);
  [DllImport("user32.dll")] public static extern bool GetClipCursor(out QuietRect r);
  [DllImport("user32.dll")] public static extern bool GetLastInputInfo(ref QuietLastInput p);
  [DllImport("kernel32.dll")] public static extern uint GetTickCount();
  public static IntPtr Find(uint pid) { IntPtr r = IntPtr.Zero;
    EnumWindows((h,l)=>{ uint p; GetWindowThreadProcessId(h,out p); var s=new StringBuilder(256); GetWindowText(h,s,256);
      if(p==pid && s.ToString().StartsWith("Triangulum")){ r=h; return false;} return true;}, IntPtr.Zero); return r; }
}
"@

$WM_CLOSE = 0x0010; $WM_SETFOCUS = 0x0007; $WM_KILLFOCUS = 0x0008; $WM_ACTIVATE = 0x0006; $WM_NCACTIVATE = 0x0086
$SW_SHOWNOACTIVATE = 4; $SW_SHOWMINNOACTIVE = 7
$SWP_NOSIZE = 0x1; $SWP_NOMOVE = 0x2; $SWP_NOZORDER = 0x4; $SWP_NOACTIVATE = 0x10

function Get-IdleSeconds {
    $l = New-Object QuietLastInput; $l.Size = 8; [void][QuietWin]::GetLastInputInfo([ref]$l)
    ([QuietWin]::GetTickCount() - $l.Time) / 1000
}

function Get-ClipText {
    $r = New-Object QuietRect; [void][QuietWin]::GetClipCursor([ref]$r)
    "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)"
}

# Start the game with --no-focus and wait until it is running frames. -Log is a
# path without an extension. Run from game/ (relative --save, --blocks and the
# like). --exit-after counts from before the renderer is made: leave it room.
function Start-QuietGame([string]$Exe, [string]$Arguments = '', [string]$Log, [int]$TimeoutSeconds = 90) {
    New-Item -ItemType Directory -Force -Path (Split-Path $Log) | Out-Null
    $fg = [QuietWin]::GetForegroundWindow()
    $clip = Get-ClipText
    # -NoNewWindow: a console of its own would take the foreground.
    $p = Start-Process -FilePath $Exe -ArgumentList "--no-focus $Arguments" -NoNewWindow -PassThru `
        -RedirectStandardOutput "$Log.out.txt" -RedirectStandardError "$Log.err.txt"
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt $TimeoutSeconds * 4; $i++) {
        Start-Sleep -Milliseconds 250
        if ($p.HasExited) { break }
        $h = [QuietWin]::Find([uint32]$p.Id)
        if ($h -ne [IntPtr]::Zero) { break }
    }
    $g = [pscustomobject]@{ Process = $p; Id = $p.Id; Hwnd = $h; Foreground = $fg; Clip = $clip; Log = $Log }
    if ($h -ne [IntPtr]::Zero) {
        Set-GameOutOfReach $g
        # The window exists long before the game listens to it: the renderer
        # is made first, and on a busy machine that takes many seconds. A call
        # made before this line is printed tests nothing.
        for ($i = 0; $i -lt $TimeoutSeconds * 4 -and -not $p.HasExited; $i++) {
            if (Select-String -Path "$Log.out.txt" -Pattern '^app: WASD' -Quiet) { break }
            Start-Sleep -Milliseconds 250
        }
        Start-Sleep 2
    }
    $g
}

# Keep the window out of the way of the person at the machine: off the screen,
# so a stray click cannot land on it, and WS_EX_NOACTIVATE, so Windows does not
# hand it the foreground when the foreground window closes or minimises (a run
# without this ended with the game in front). winit rewrites the window's styles
# now and then, so every call below that changes the window sets this again.
function Set-GameOutOfReach($g) {
    if ($g.Hwnd -eq [IntPtr]::Zero) { return }
    $ex = [QuietWin]::GetWindowLongPtr($g.Hwnd, -20)
    [void][QuietWin]::SetWindowLongPtr($g.Hwnd, -20, [IntPtr]($ex.ToInt64() -bor 0x08000000))
    [void][QuietWin]::SetWindowPos($g.Hwnd, [IntPtr]::Zero, -32000, -32000, 0, 0, $SWP_NOSIZE -bor $SWP_NOZORDER -bor $SWP_NOACTIVATE)
}

function Test-GameAlive($g) { $null -ne (Get-Process -Id $g.Id -ErrorAction SilentlyContinue) }
function Get-GameCpu($g) { (Get-Process -Id $g.Id -ErrorAction SilentlyContinue).CPU }
function Test-GameMinimised($g) { [QuietWin]::IsIconic($g.Hwnd) }

function Get-GameClientSize($g) {
    $r = New-Object QuietRect; [void][QuietWin]::GetClientRect($g.Hwnd, [ref]$r)
    "$($r.Right - $r.Left)x$($r.Bottom - $r.Top)"
}

function Hide-Game($g) { [void][QuietWin]::ShowWindow($g.Hwnd, $SW_SHOWMINNOACTIVE) }
function Show-Game($g) { [void][QuietWin]::ShowWindow($g.Hwnd, $SW_SHOWNOACTIVATE); Set-GameOutOfReach $g }

function Set-GameSize($g, [int]$Width, [int]$Height) {
    [void][QuietWin]::SetWindowPos($g.Hwnd, [IntPtr]::Zero, 0, 0, $Width, $Height, $SWP_NOMOVE -bor $SWP_NOZORDER -bor $SWP_NOACTIVATE)
    Set-GameOutOfReach $g
}

# The window thinks it gained or lost focus. These are posted to its queue, so
# nothing happens to the real focus; winit reports Focused(true) only when it
# has seen both messages, and Focused(false) on either.
function Send-GameFocus($g, [bool]$On) {
    $w = [IntPtr]::new([int]$On)
    if ($On) {
        [void][QuietWin]::PostMessage($g.Hwnd, $WM_NCACTIVATE, $w, [IntPtr]::Zero)
        [void][QuietWin]::PostMessage($g.Hwnd, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    } else {
        [void][QuietWin]::PostMessage($g.Hwnd, $WM_KILLFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][QuietWin]::PostMessage($g.Hwnd, $WM_NCACTIVATE, $w, [IntPtr]::Zero)
    }
}

function Send-GameActivate($g, [bool]$On) {
    [void][QuietWin]::PostMessage($g.Hwnd, $WM_ACTIVATE, [IntPtr]::new([int]$On), [IntPtr]::Zero)
}

# Did the game disturb anything? Throws if it has the foreground or the mouse
# clip changed since it started. Whoever is at the machine moves the foreground
# about, so the only foreground that counts is the game's. A throw leaves the
# game running: call Stop-QuietGame in a finally.
function Assert-Quiet($g) {
    $fg = [QuietWin]::GetForegroundWindow()
    $clip = Get-ClipText
    if ($fg -eq $g.Hwnd) { throw "the game has the foreground" }
    if ($clip -ne $g.Clip) { throw "the mouse clip changed: was $($g.Clip), now $clip" }
    "quiet: foreground $fg (was $($g.Foreground)), clip $clip"
}

# Close it the way the X does, wait for it to save and go, and kill it only if
# it stays. Kills nothing but the process this tool started.
function Stop-QuietGame($g, [int]$TimeoutSeconds = 20) {
    if (Test-GameAlive $g) {
        [void][QuietWin]::PostMessage($g.Hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        for ($i = 0; $i -lt $TimeoutSeconds * 4 -and (Test-GameAlive $g); $i++) { Start-Sleep -Milliseconds 250 }
    }
    if (Test-GameAlive $g) { Stop-Process -Id $g.Id -Force; 'stuck: killed' } else { 'closed' }
}
