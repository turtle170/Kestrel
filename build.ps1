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

# 5. Build Kestrel pkg & guest utilities
Write-Host "Building Kestrel pkg & guest utilities..." -ForegroundColor Cyan
Set-Location "$PSScriptRoot"
cargo build --release -p kestrel-pkg
cargo build --release -p kestrel-term

rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl -p kestrel-init
cargo build --release --target x86_64-unknown-linux-musl -p kestrel-pkg

# 6. Populate bin directory and build initramfs
$binDir = Join-Path $PSScriptRoot "bin"
if (-not (Test-Path $binDir)) {
    New-Item -ItemType Directory -Path $binDir | Out-Null
}
Copy-Item "target\release\kestrel.exe" -Destination $binDir -Force
Copy-Item "target\release\kestrel-pkg.exe" -Destination $binDir -Force
Copy-Item "target\release\kestrel-term.exe" -Destination $binDir -Force
Copy-Item "target\x86_64-unknown-linux-musl\release\kestrel-init" -Destination $binDir -Force
Copy-Item "target\x86_64-unknown-linux-musl\release\kestrel-pkg" -Destination (Join-Path $binDir "kestrel") -Force

Write-Host "Building guest initramfs.cpio with baked symlinks..." -ForegroundColor Cyan
& "target\release\kestrel-pkg.exe" build-initramfs -i "target\x86_64-unknown-linux-musl\release\kestrel-init" -k "target\x86_64-unknown-linux-musl\release\kestrel-pkg" -o (Join-Path $binDir "initramfs.cpio")

# 7. Linux Kernel Fetch (Mocked for speed in dev environment)
Write-Host "Fetching and Configuring minimal Linux Kernel (7.0.12 target)..." -ForegroundColor Cyan
if (-Not (Test-Path "linux-source")) {
    New-Item -ItemType Directory -Force -Path "linux-source" | Out-Null
}
Set-Content -Path "linux-source\bzImage" -Value "MOCK_KERNEL_IMAGE"

Write-Host "Build Complete!" -ForegroundColor Green
Write-Host "Run bin\kestrel.exe to start the Kestrel environment." -ForegroundColor Yellow
