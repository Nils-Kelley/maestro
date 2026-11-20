# Capture the app's own window to shot.png in the repo folder. -Launch starts the release build first; -Keys "{TAB}|text" sends keys before the capture.
# Never use this while someone is working in the app: it takes the keyboard focus.
param([switch]$Launch,[string]$Keys,[int]$Wait=0)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@
$dlg=[W]::FindWindow('#32770','\\'); if ($dlg -ne [IntPtr]::Zero) { [void][W]::PostMessage($dlg,0x0010,[IntPtr]::Zero,[IntPtr]::Zero); "closed the error box" }
if ($Launch) { Get-Process maestro -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Process (Join-Path $PSScriptRoot '..\src-tauri\target\release\maestro.exe') -WorkingDirectory (Join-Path $PSScriptRoot '..'); Start-Sleep -Seconds 9 }
$p=Get-Process maestro -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if ($p -and $Keys) { $sh=New-Object -ComObject WScript.Shell; [void]$sh.AppActivate($p.Id); Start-Sleep -Milliseconds 400; foreach ($k in $Keys.Split('|')) { $sh.SendKeys($k); Start-Sleep -Milliseconds 250 }; Start-Sleep -Milliseconds $Wait }
if (-not $p) { "maestro is not running with a window"; exit }
$r=New-Object W+RECT; [void][W]::GetWindowRect($p.MainWindowHandle,[ref]$r)
$w=$r.R-$r.L; $h=$r.B-$r.T; "window ${w}x${h} at $($r.L),$($r.T) visible=$([W]::IsWindowVisible($p.MainWindowHandle))"
$bmp=New-Object System.Drawing.Bitmap $w,$h; $g=[System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($r.L,$r.T,0,0,(New-Object System.Drawing.Size $w,$h)); $bmp.Save((Join-Path $PSScriptRoot '..\shot.png'))
