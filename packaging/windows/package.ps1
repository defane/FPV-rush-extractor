# Native Windows x64 build. Requires Rust, NSIS and prepared FFmpeg resources.
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot | Split-Path -Parent
$VersionLine = Get-Content "$Root/fpv-rs/Cargo.toml" | Select-String '^version = "([^"]+)"' | Select-Object -First 1
$Version = $VersionLine.Matches.Groups[1].Value
$Dist = Join-Path $Root 'dist'
$Stage = Join-Path $Root 'fpv-rs/target/windows-package'
foreach ($file in @('bin/ffmpeg.exe', 'bin/ffprobe.exe', 'licenses/ffmpeg/ffmpeg-8.1.3.tar.xz')) {
    if (!(Test-Path "$Root/packaging/resources/$file")) { throw "Missing FFmpeg resource: $file" }
}
Push-Location "$Root/fpv-rs"
try {
    cargo build --release --locked -p fpv-app
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
} finally { Pop-Location }
New-Item -Force -ItemType Directory $Stage, $Dist | Out-Null
Copy-Item "$Root/fpv-rs/target/release/fpv-rush.exe" "$Stage/fpv-rush.exe" -Force
Copy-Item "$Root/packaging/resources" "$Stage/" -Recurse -Force
Copy-Item "$Root/LICENSE-MIT", "$Root/LICENSE-APACHE" "$Stage/" -Force
$Zip = "$Dist/FPV-Rush-$Version-windows-x64.zip"
Compress-Archive -Path "$Stage/*" -DestinationPath $Zip -Force
$Makensis = Get-Command makensis -ErrorAction SilentlyContinue
if (!$Makensis) { $Makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS/makensis.exe' }
& $Makensis "/DVERSION=$Version" "/DSTAGE=$Stage" "/DDIST=$Dist" "$PSScriptRoot/installer.nsi"
if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed' }
& "$Stage/fpv-rush.exe" --version
if ($LASTEXITCODE -ne 0) { throw 'Packaged executable failed its smoke test' }
