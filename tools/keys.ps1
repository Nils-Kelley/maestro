# Send keys to the running app: -Keys "{TAB}|text" (parts separated by |). It takes the keyboard focus: not while someone is using the app.
param([string]$Keys)
$p=Get-Process maestro -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { "not running"; exit }
$sh=New-Object -ComObject WScript.Shell; [void]$sh.AppActivate($p.Id); Start-Sleep -Milliseconds 500
foreach ($k in $Keys.Split('|')) { $sh.SendKeys($k); Start-Sleep -Milliseconds 350 }
