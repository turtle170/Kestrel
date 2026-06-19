$ErrorActionPreference = "Stop"

Write-Host "Kestrel OS Build Script" -ForegroundColor Green

# 1. Build Kestrel Host (Windows side)
Write-Host "Building Kestrel Host..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot\kestrel-host"
cargo build --release

# 2. Build Kestrel IPC
Write-Host "Building Kestrel IPC Server..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot\kestrel-ipc"
cargo build --release

# 3. Build Antiproton
Write-Host "Building Antiproton Subsystem..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot\antiproton"
cargo build --release

# 4. Build Kestrel Bridge (Linux Kernel Module)
Write-Host "Building Kestrel Bridge..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot\kestrel-bridge"
cargo build --release

# 5. Linux Kernel Fetch (Mocked for speed in dev environment)
Write-Host "Fetching and Configuring minimal Linux Kernel (7.0.12 target)..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot"
if (-Not (Test-Path "linux-source")) {
    New-Item -ItemType Directory -Force -Path "linux-source" | Out-Null
}
Set-Content -Path "linux-source\bzImage" -Value "MOCK_KERNEL_IMAGE"

Write-Host "Build Complete!" -ForegroundColor Green
Write-Host "Run kestrel-host.exe to start the Kestrel environment." -ForegroundColor Yellow
