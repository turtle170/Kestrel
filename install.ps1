# Kestrel OS Installer for Windows
# Usage: irm https://raw.githubusercontent.com/turtle170/Kestrel/master/install.ps1 | iex

$ErrorActionPreference = "Stop"

Write-Host "================================================================" -ForegroundColor Cyan
Write-Host "                     Installing Kestrel OS                      " -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan

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

# 4. Fetch Precompiled Binaries from GitHub
Push-Location $installDir
$binDir = Join-Path $installDir "bin"
if (-not (Test-Path $binDir)) {
    New-Item -ItemType Directory -Path $binDir | Out-Null
}

$repoUrl = "https://github.com/turtle170/Kestrel/releases/download/latest"
$filesToDownload = @(
    "kestrel.exe",
    "kestrel-pkg.exe",
    "kestrel-term.exe",
    "initramfs.cpio",
    "kestrel",
    "kestrel-init"
)

Write-Host "[Installer] Downloading precompiled Kestrel release binaries..." -ForegroundColor White
foreach ($file in $filesToDownload) {
    $source = "$repoUrl/$file"
    $dest = Join-Path $binDir $file
    Write-Host "[Installer] Downloading $file..." -ForegroundColor Gray
    try {
        Invoke-WebRequest -Uri $source -OutFile $dest -UseBasicParsing
    } catch {
        Write-Error "Failed to download $file from $source. Please check your internet connection."
    }
}

# Copy icon if present
if (Test-Path "icon.png") {
    Copy-Item "icon.png" -Destination $binDir -Force
}

# 6. Add to user Path
Write-Host "[Installer] Updating User Environment PATH..." -ForegroundColor White
$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
if (($userPath -split ';') -contains $binDir) {
    Write-Host "[Installer] PATH already contains Kestrel." -ForegroundColor Green
} else {
    $newUserPath = $userPath + ";$binDir"
    [Environment]::SetEnvironmentVariable("Path", $newUserPath, [EnvironmentVariableTarget]::User)
    $env:Path += ";$binDir"
    Write-Host "[Installer] Added Kestrel to user PATH successfully." -ForegroundColor Green
}

Pop-Location

Write-Host "`n[Success] Kestrel OS installed successfully at $installDir!" -ForegroundColor Green
Write-Host "Open a new terminal session and run:" -ForegroundColor White
Write-Host "  kestrel --help" -ForegroundColor Cyan
Write-Host "  kestrel-pkg --help" -ForegroundColor Cyan
