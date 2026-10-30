<#
.SYNOPSIS
    Builds the Local-AI-Route Tauri v2 Windows x64 release package.
#>

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$projectRoot = Split-Path -Parent $scriptDir

Set-Location $projectRoot

Write-Host "==> Ensuring build tools in PATH..." -ForegroundColor Cyan
if (Test-Path "$HOME\w64devkit\bin") {
    $env:PATH = "$HOME\w64devkit\bin;$env:PATH"
}
if (Test-Path "$HOME\.cargo\bin") {
    $env:PATH = "$HOME\.cargo\bin;$env:PATH"
}

Write-Host "==> Installing Node.js dependencies..." -ForegroundColor Cyan
pnpm install

Write-Host "==> Building frontend assets..." -ForegroundColor Cyan
pnpm build

Write-Host "==> Compiling Tauri v2 Windows release binary..." -ForegroundColor Cyan
pnpm tauri build

$releaseExe = Join-Path $projectRoot "src-tauri\target\release\local-ai-route.exe"

if (Test-Path $releaseExe) {
    Write-Host "`n✔ Build successful!" -ForegroundColor Green
    Write-Host "Executable: $releaseExe" -ForegroundColor Green

    $distDir = Join-Path $projectRoot "release_dist"
    if (Test-Path $distDir) { Remove-Item -Recurse -Force $distDir }
    New-Item -ItemType Directory -Path $distDir | Out-Null

    if (Get-Command strip -ErrorAction SilentlyContinue) {
        Write-Host "==> Stripping symbols for release..." -ForegroundColor Cyan
        & strip $releaseExe
    }

    Copy-Item $releaseExe (Join-Path $distDir "Local-AI-Route.exe")

    $loaderDll = Join-Path $projectRoot "src-tauri\target\release\WebView2Loader.dll"
    if (Test-Path $loaderDll) {
        Copy-Item $loaderDll (Join-Path $distDir "WebView2Loader.dll")
        Write-Host "✔ Included WebView2Loader.dll in distribution" -ForegroundColor Green
    }

    New-Item -ItemType File -Path (Join-Path $distDir "portable.flag") | Out-Null
    $dataDir = Join-Path $distDir "data"
    New-Item -ItemType Directory -Path $dataDir | Out-Null
    New-Item -ItemType File -Path (Join-Path $dataDir ".gitkeep") | Out-Null

    $zipFile = Join-Path $projectRoot "Local-AI-Route-v0.2.0-windows-x64-portable.zip"
    if (Test-Path $zipFile) { Remove-Item -Force $zipFile }
    Compress-Archive -Path "$distDir\*" -DestinationPath $zipFile

    Write-Host "✔ Created Portable ZIP: $zipFile" -ForegroundColor Green
} else {
    Write-Error "Release executable not found at $releaseExe"
}
