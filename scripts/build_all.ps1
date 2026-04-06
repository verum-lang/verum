# Comprehensive build script for Verum (Windows PowerShell)
# Builds all crates with proper error handling and diagnostics

param(
    [switch]$Clean,
    [switch]$Release,
    [switch]$Debug,
    [switch]$Test,
    [switch]$NoTest,
    [switch]$Bench,
    [switch]$Docs,
    [int]$Jobs = 0,
    [switch]$Help
)

$ErrorActionPreference = "Continue" # Don't stop on warnings

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Green
}

function Write-Warning-Custom {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Write-Error-Custom {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

function Write-Step {
    param([string]$Message)
    Write-Host "[STEP] $Message" -ForegroundColor Blue
}

function Print-Usage {
    Write-Host "Usage: .\build_all.ps1 [OPTIONS]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Clean          Clean build artifacts before building"
    Write-Host "  -Release        Build in release mode (default)"
    Write-Host "  -Debug          Build in debug mode"
    Write-Host "  -Test           Run tests after building (default)"
    Write-Host "  -NoTest         Skip tests"
    Write-Host "  -Bench          Run benchmarks"
    Write-Host "  -Docs           Generate documentation"
    Write-Host "  -Jobs N         Number of parallel jobs (default: auto)"
    Write-Host "  -Help           Show this help message"
    Write-Host ""
    Write-Host "Examples:"
    Write-Host "  .\build_all.ps1                    # Build in release mode with tests"
    Write-Host "  .\build_all.ps1 -Debug -Test       # Build in debug mode with tests"
    Write-Host "  .\build_all.ps1 -Clean -Release -Bench -Docs  # Full build"
}

function Check-Dependencies {
    Write-Step "Checking dependencies..."

    $missingDeps = @()

    if (!(Get-Command rustc -ErrorAction SilentlyContinue)) {
        $missingDeps += "rust"
    }

    if (!(Get-Command llvm-config -ErrorAction SilentlyContinue)) {
        $missingDeps += "llvm"
    }

    if (!(Get-Command z3 -ErrorAction SilentlyContinue)) {
        $missingDeps += "z3"
    }

    if ($missingDeps.Count -gt 0) {
        Write-Error-Custom "Missing dependencies: $($missingDeps -join ', ')"
        Write-Error-Custom "Run '.\scripts\setup.ps1' to install all dependencies"
        return $false
    }

    Write-Info "✓ All dependencies are available"

    # Print versions
    Write-Info "Rust: $(rustc --version)"
    try { Write-Info "LLVM: $(llvm-config --version)" } catch { Write-Warning-Custom "Could not determine LLVM version" }
    try { Write-Info "Z3: $((z3 --version) -split "`n" | Select-Object -First 1)" } catch { Write-Warning-Custom "Could not determine Z3 version" }

    return $true
}

function Clean-Build {
    Write-Step "Cleaning build artifacts..."
    cargo clean
    Write-Info "✓ Clean complete"
}

function Build-Workspace {
    param([string]$BuildType, [int]$ParallelJobs)

    Write-Step "Building Verum workspace..."

    $buildFlags = @()

    if ($BuildType -eq "release") {
        $buildFlags += "--release"
        Write-Info "Building in RELEASE mode"
    } else {
        Write-Info "Building in DEBUG mode"
    }

    if ($ParallelJobs -gt 0) {
        $buildFlags += "-j", $ParallelJobs
        Write-Info "Parallel jobs: $ParallelJobs"
    }

    Write-Info "Build flags: $($buildFlags -join ' ')"

    # Build with proper error handling
    $buildOutput = cargo build $buildFlags 2>&1 | Tee-Object -FilePath "build.log"

    if ($LASTEXITCODE -eq 0) {
        Write-Info "✓ Build successful"
        return $true
    } else {
        Write-Error-Custom "✗ Build failed"
        Write-Error-Custom "Check build.log for details"
        return $false
    }
}

function Run-Tests {
    param([string]$BuildType, [int]$ParallelJobs, [bool]$RunTests)

    if (!$RunTests) {
        Write-Info "Skipping tests"
        return $true
    }

    Write-Step "Running tests..."

    $testFlags = @("--workspace")

    if ($BuildType -eq "release") {
        $testFlags += "--release"
    }

    if ($ParallelJobs -gt 0) {
        $testFlags += "-j", $ParallelJobs
    }

    $testOutput = cargo test $testFlags 2>&1 | Tee-Object -FilePath "test.log"

    if ($LASTEXITCODE -eq 0) {
        Write-Info "✓ All tests passed"
        return $true
    } else {
        Write-Error-Custom "✗ Some tests failed"
        Write-Error-Custom "Check test.log for details"
        return $false
    }
}

function Run-Benches {
    param([bool]$RunBenches)

    if (!$RunBenches) {
        Write-Info "Skipping benchmarks"
        return $true
    }

    Write-Step "Running benchmarks..."

    $benchOutput = cargo bench --workspace 2>&1 | Tee-Object -FilePath "bench.log"

    if ($LASTEXITCODE -eq 0) {
        Write-Info "✓ Benchmarks complete"
        return $true
    } else {
        Write-Warning-Custom "Some benchmarks failed"
        return $false
    }
}

function Generate-Docs {
    param([bool]$GenDocs)

    if (!$GenDocs) {
        Write-Info "Skipping documentation"
        return $true
    }

    Write-Step "Generating documentation..."

    $docOutput = cargo doc --workspace --no-deps 2>&1 | Tee-Object -FilePath "doc.log"

    if ($LASTEXITCODE -eq 0) {
        Write-Info "✓ Documentation generated"
        Write-Info "Open: target\doc\verum_cli\index.html"
        return $true
    } else {
        Write-Warning-Custom "Documentation generation had warnings"
        return $true # Don't fail on doc warnings
    }
}

function Print-BuildSummary {
    param([string]$BuildType, [int]$ParallelJobs, [bool]$RunTests, [bool]$RunBenches, [bool]$GenDocs)

    Write-Host ""
    Write-Info "=== Build Summary ==="
    Write-Host "Build type: $BuildType"
    Write-Host "Parallel jobs: $(if ($ParallelJobs -gt 0) { $ParallelJobs } else { 'auto' })"
    Write-Host "Tests: $(if ($RunTests) { 'yes' } else { 'no' })"
    Write-Host "Benchmarks: $(if ($RunBenches) { 'yes' } else { 'no' })"
    Write-Host "Documentation: $(if ($GenDocs) { 'yes' } else { 'no' })"
    Write-Host ""

    $binaryPath = if ($BuildType -eq "release") { "target\release\verum.exe" } else { "target\debug\verum.exe" }

    if (Test-Path $binaryPath) {
        Write-Info "Binary location: $binaryPath"
        $size = (Get-Item $binaryPath).Length / 1MB
        Write-Info "Binary size: $([math]::Round($size, 2)) MB"
    }

    Write-Host ""
}

function Main {
    if ($Help) {
        Print-Usage
        return
    }

    Write-Info "=== Verum Build Script (Windows) ==="
    Write-Host ""

    # Check if we're in the right directory
    if (!(Test-Path "Cargo.toml")) {
        Write-Error-Custom "Cargo.toml not found. Please run this script from the project root."
        exit 1
    }

    # Determine build type
    $buildType = if ($Debug) { "debug" } else { "release" }

    # Determine if tests should run
    $runTests = if ($NoTest) { $false } else { $true }

    # Auto-detect parallel jobs if not specified
    if ($Jobs -eq 0) {
        $Jobs = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
    }

    # Check dependencies
    if (!(Check-Dependencies)) {
        exit 1
    }

    # Clean if requested
    if ($Clean) {
        Clean-Build
    }

    # Build
    Write-Host ""
    if (!(Build-Workspace -BuildType $buildType -ParallelJobs $Jobs)) {
        Write-Error-Custom "Build failed"
        exit 1
    }

    # Run tests
    Write-Host ""
    if (!(Run-Tests -BuildType $buildType -ParallelJobs $Jobs -RunTests $runTests)) {
        Write-Error-Custom "Tests failed"
        exit 1
    }

    # Run benchmarks
    Write-Host ""
    Run-Benches -RunBenches $Bench

    # Generate documentation
    Write-Host ""
    Generate-Docs -GenDocs $Docs

    # Print summary
    Print-BuildSummary -BuildType $buildType -ParallelJobs $Jobs -RunTests $runTests -RunBenches $Bench -GenDocs $Docs

    Write-Info "=== Build Complete ==="
}

Main
