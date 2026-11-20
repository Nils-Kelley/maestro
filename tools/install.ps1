# Put the release build where Windows expects an app: %LOCALAPPDATA%\Programs\Maestro, with a Start Menu entry.
# Run it after `cargo build --release` (or tools/build.sh). Your project list (maestro.config.json in that folder) is never replaced.
$ErrorActionPreference = 'Stop'   # a copy that fails must not end in "installed"
$src = Join-Path $PSScriptRoot '..'
$rel = Join-Path $src 'src-tauri\target\release'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Maestro'
if (-not (Test-Path (Join-Path $rel 'maestro.exe'))) { throw "no build at $rel\maestro.exe: run cargo build --release in src-tauri first" }
New-Item -ItemType Directory -Force -Path $dir | Out-Null
# stop the app and everything it started (a stuck copy keeps the file open), then wait until the file is really free
# through cmd, so that "no such process" on its error stream is not taken for a failure of this script
& cmd.exe /c 'taskkill /F /T /IM Maestro.exe >nul 2>&1'
for ($i = 0; $i -lt 40 -and (Get-Process maestro -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 250 }
$target = Join-Path $dir 'Maestro.exe'
for ($i = 0; $i -lt 40 -and (Test-Path $target); $i++) { try { $f = [IO.File]::Open($target, 'Open', 'ReadWrite', 'None'); $f.Close(); break } catch { Start-Sleep -Milliseconds 250 } }
Copy-Item (Join-Path $rel 'maestro.exe') $target -Force
# a build made with the GNU toolchain loads the browser layer through this library and shows no window without it;
# a build made with MSVC has it built in and there is no such file
$loader = Join-Path $rel 'WebView2Loader.dll'
if (Test-Path $loader) { Copy-Item $loader $dir -Force }
# a project list kept beside the source (from running the build in place) comes along the first time only
$cfg = Join-Path $dir 'maestro.config.json'
$mine = Join-Path $src 'maestro.config.json'
if (-not (Test-Path $cfg) -and (Test-Path $mine)) { Copy-Item $mine $cfg }
$sh = New-Object -ComObject WScript.Shell
$lnk = $sh.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Maestro.lnk'))
$lnk.TargetPath = $target; $lnk.WorkingDirectory = $dir; $lnk.Description = 'Your agents and their harness, in one place'; $lnk.Save()
"installed to $dir"
