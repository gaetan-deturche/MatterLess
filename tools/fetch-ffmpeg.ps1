# Fetches the ffmpeg build the video player loads -- the exact one its struct
# bindings were generated from -- into third_party/ffmpeg, and checks that it
# is that build. Run by the release workflow, and by hand on a new checkout:
#
#   pwsh tools/fetch-ffmpeg.ps1
#
# LGPL, shared: the DLLs stay separate files, and nothing GPL or non-free is in
# them. A debug build finds them here; the installer puts them beside the app.

$ErrorActionPreference = "Stop"

$tag = "autobuild-2026-09-24-14-14"
$name = "ffmpeg-n9.0.2-3-ga5923073bf-win64-lgpl-shared-9.0"
$sha256 = "735bae484ba2c3342bfb34df477b9c6b0f43f9819f4d2fde011be293ee1b6517"

$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "third_party\ffmpeg"
if (Test-Path (Join-Path $dest "bin\avcodec-63.dll")) {
    "ffmpeg is already in $dest"
    return
}

$zip = Join-Path ([System.IO.Path]::GetTempPath()) "$name.zip"
Invoke-WebRequest -Uri "https://github.com/BtbN/FFmpeg-Builds/releases/download/$tag/$name.zip" -OutFile $zip
$got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($got -ne $sha256) {
    throw "the ffmpeg download is not the pinned build (sha256 $got)"
}

$unpack = Join-Path ([System.IO.Path]::GetTempPath()) "matterless-ffmpeg"
if (Test-Path $unpack) { Remove-Item -Recurse -Force $unpack }
Expand-Archive $zip -DestinationPath $unpack
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
Move-Item (Join-Path $unpack $name) $dest
Remove-Item $zip
"ffmpeg $name is in $dest"
