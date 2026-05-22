#!/usr/bin/env pwsh
# Automated Windows build entrypoint for Verum.
#
# Steps performed:
#   1. Locate Visual Studio and load the MSVC toolchain (cmake / cl / link).
#   2. Ensure ninja is available (auto-installed via winget if missing).
#   3. Build the local LLVM tree (llvm/install/) if absent — this MUST happen
#      before cargo, because .cargo/config.toml selects `lld-link` (shipped in
#      llvm/install/bin) as the linker for *every* crate. build.rs's own LLVM
#      auto-build runs too late: the linker is needed from the first crate on.
#   4. Run `cargo build` with llvm/install/bin on PATH.
#
# A fresh checkout takes 1-2 h and ~50 GB of disk (LLVM + Z3 are built from
# source). Subsequent builds reuse llvm/install/ and are far faster.
#
# Prerequisites (the script verifies and reports each):
#   - Visual Studio 2019+ with the "Desktop development with C++" workload
#   - Git for Windows (provides bash, required by llvm/build.sh)
#   - winget (used to auto-install ninja if missing)
#
# Usage:
#   .\build-windows.ps1                      # release build
#   .\build-windows.ps1 -DebugBuild          # debug build
#   .\build-windows.ps1 -CargoArgs -p verum_cli   # extra args passed to cargo

[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = 'Stop'

function Find-VsInstall {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) {
        throw "vswhere.exe not found. Install Visual Studio 2019+ with the 'Desktop development with C++' workload."
    }
    $path = & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath
    if (-not $path) {
        throw "No Visual Studio with the MSVC C++ toolset found. Install the 'Desktop development with C++' workload."
    }
    return $path
}

Write-Host "==> Locating Visual Studio..." -ForegroundColor Cyan
$vs = Find-VsInstall
Write-Host "    $vs"

Write-Host "==> Loading MSVC environment (x64)..." -ForegroundColor Cyan
Import-Module "$vs\Common7\Tools\Microsoft.VisualStudio.DevShell.dll"
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation `
    -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null

# ninja is mandatory: llvm/build.sh uses it as the CMake generator on Windows
# (the "Unix Makefiles" fallback needs `make`, which MSVC does not ship).
if (-not (Get-Command ninja -ErrorAction SilentlyContinue)) {
    Write-Host "==> ninja not found - installing via winget..." -ForegroundColor Yellow
    winget install --id Ninja-build.Ninja `
        --accept-source-agreements --accept-package-agreements --disable-interactivity
    $env:Path = "$env:LOCALAPPDATA\Microsoft\WinGet\Links;$env:Path"
}

# bash is required: build.rs invokes llvm/build.sh through it on Windows.
if (-not (Get-Command bash -ErrorAction SilentlyContinue)) {
    throw "bash not found. Install Git for Windows so the local LLVM build (llvm/build.sh) can run."
}

# --- Local LLVM ----------------------------------------------------------
# .cargo/config.toml sets `linker = "lld-link"` for the msvc target. lld-link
# is produced by the local LLVM build and lives in llvm/install/bin. It must
# be on PATH before cargo runs, so the LLVM build cannot be deferred to
# build.rs (which only runs once verum_llvm_sys is reached).
$repo        = $PSScriptRoot
$llvmInstall = Join-Path $repo 'llvm\install'
$llvmConfig  = Join-Path $llvmInstall 'bin\llvm-config.exe'

if (-not (Test-Path $llvmConfig)) {
    Write-Host "==> Local LLVM not found - building from source (llvm/build.sh)." -ForegroundColor Yellow
    Write-Host "    Clones llvm-project and builds LLVM 21 + LLD + MLIR." -ForegroundColor Yellow
    Write-Host "    Expect 1-2 h and ~50 GB of disk on a fresh checkout." -ForegroundColor Yellow
    Push-Location (Join-Path $repo 'llvm')
    try {
        & bash ./build.sh
        if ($LASTEXITCODE -ne 0) {
            throw "llvm/build.sh failed with exit code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }
    if (-not (Test-Path $llvmConfig)) {
        throw "llvm/build.sh finished but $llvmConfig is missing."
    }
}
else {
    Write-Host "==> Local LLVM present: $llvmInstall" -ForegroundColor Cyan
}

# Put lld-link (and the rest of the LLVM tools) on PATH for cargo/rustc.
$env:Path = "$llvmInstall\bin;$env:Path"

Write-Host "==> Toolchain:" -ForegroundColor Cyan
foreach ($t in 'cmake', 'cl', 'ninja', 'bash', 'git', 'cargo', 'lld-link') {
    $c = Get-Command $t -ErrorAction SilentlyContinue
    Write-Host ("    {0,-9} {1}" -f $t, $(if ($c) { $c.Source } else { 'MISSING' }))
}

# Build an explicit argument array. Splatting (@var) is avoided on purpose:
# a single-element array unwraps to a scalar string, and splatting a string
# passes it character-by-character to the native command.
$cargoArgv = [System.Collections.Generic.List[string]]::new()
$cargoArgv.Add('build')
if (-not $DebugBuild) { $cargoArgv.Add('--release') }
if ($CargoArgs) { $CargoArgs | ForEach-Object { $cargoArgv.Add($_) } }
Write-Host "==> cargo $($cargoArgv -join ' ')" -ForegroundColor Cyan
& cargo $cargoArgv.ToArray()
$code = $LASTEXITCODE
Write-Host "==> cargo build exit code: $code" -ForegroundColor $(if ($code -eq 0) { 'Green' } else { 'Red' })
exit $code
