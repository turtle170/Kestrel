# Kestrel OS Installer for Windows
# Usage: irm https://raw.githubusercontent.com/turtle170/Kestrel/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

Write-Host "╔══════════════════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "║                    Installing Kestrel OS                     ║" -ForegroundColor Cyan
Write-Host "╚══════════════════════════════════════════════════════════════╝" -ForegroundColor Cyan

# 1. Determine Target Directory
$installDir = "D:\Kestrel"
if (-not (Test-Path "D:")) {
    $installDir = "$env:USERPROFILE\Kestrel"
}
Write-Host "[Installer] Installation directory chosen: $installDir" -ForegroundColor Green

# 2. Check Prerequisites
Write-Host "[Installer] Verifying tools..." -ForegroundColor White
$gitCheck = Get-Command git -ErrorAction SilentlyContinue
if (-not $gitCheck) {
    Write-Error "Git is required to install Kestrel OS. Please install Git and try again."
}
$cargoCheck = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargoCheck) {
    Write-Error "Rust/Cargo is required to compile Kestrel OS. Please install rustup (https://rustup.rs) and try again."
}

# 3. Clone / Update Repository
if (Test-Path $installDir) {
    Write-Host "[Installer] Directory exists. Pulling latest code..." -ForegroundColor White
    Push-Location $installDir
    git pull
    Pop-Location
} else {
    Write-Host "[Installer] Cloning Kestrel repository..." -ForegroundColor White
    git clone https://github.com/turtle170/Kestrel.git $installDir
}

# 4. Compile Binaries
Push-Location $installDir

Write-Host "[Installer] Adding target 'x86_64-unknown-linux-musl' for guest kernel..." -ForegroundColor White
rustup target add x86_64-unknown-linux-musl

Write-Host "[Installer] Building Kestrel Linux guest utilities..." -ForegroundColor White
$env:RUSTFLAGS="-C linker-flavor=ld.lld -C linker=rust-lld"
cargo build --release --target x86_64-unknown-linux-musl -p kestrel-init
cargo build --release --target x86_64-unknown-linux-musl -p kestrel-pkg
$env:RUSTFLAGS=""

Write-Host "[Installer] Building Kestrel Windows host and tools..." -ForegroundColor White
cargo build --release --workspace --exclude kestrel-init --exclude kestrel-bridge --exclude beak-fs

# 5. Populate Bin Directory
$binDir = Join-Path $installDir "bin"
if (-not (Test-Path $binDir)) {
    New-Item -ItemType Directory -Path $binDir | Out-Null
}

Write-Host "[Installer] Installing binaries..." -ForegroundColor White
Copy-Item "target\release\kestrel.exe" -Destination $binDir -Force
Copy-Item "target\release\kestrel-pkg.exe" -Destination $binDir -Force
Copy-Item "target\release\kestrel-term.exe" -Destination $binDir -Force
Copy-Item "target\x86_64-unknown-linux-musl\release\kestrel-init" -Destination $binDir -Force
Copy-Item "target\x86_64-unknown-linux-musl\release\kestrel-pkg" -Destination (Join-Path $binDir "kestrel") -Force

# Copy icon if present
if (Test-Path "icon.png") {
    Copy-Item "icon.png" -Destination $binDir -Force
}

# 6. Add to user Path
Write-Host "[Installer] Updating User Environment PATH..." -ForegroundColor White
$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
if ($userPath -notsplit ';' -contains $binDir) {
    Write-Host "[Installer] PATH already contains Kestrel." -ForegroundColor Green
} else {
    $newUserPath = $userPath + ";$binDir"
    [Environment]::SetEnvironmentVariable("Path", $newUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";$binDir"
    Write-Host "[Installer] Added Kestrel to user PATH successfully." -ForegroundColor Green
}

Pop-Location

Write-Host "`n🎉 Kestrel OS installed successfully at $installDir!" -ForegroundColor Green
Write-Host "Open a new terminal session and run:" -ForegroundColor White
Write-Host "  kestrel --help" -ForegroundColor Cyan
Write-Host "  kestrel-pkg --help" -ForegroundColor Cyan
