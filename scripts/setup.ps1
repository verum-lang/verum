# Cross-platform setup script for Verum (Windows PowerShell)
# Installs all required dependencies for building Verum

param(
    [switch]$Force
)

$ErrorActionPreference = "Stop"

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

function Test-CommandExists {
    param([string]$Command)
    try {
        if (Get-Command $Command -ErrorAction Stop) {
            Write-Info "✓ $Command is installed"
            return $true
        }
    } catch {
        Write-Warning-Custom "✗ $Command is not installed"
        return $false
    }
}

function Install-Rust {
    Write-Info "Installing Rust toolchain..."

    if (!(Test-CommandExists "rustc")) {
        Write-Info "Downloading rustup-init..."
        $rustupUrl = "https://win.rustup.rs/x86_64"
        $rustupPath = "$env:TEMP\rustup-init.exe"

        Invoke-WebRequest -Uri $rustupUrl -OutFile $rustupPath

        Write-Info "Running rustup installer..."
        Start-Process -FilePath $rustupPath -ArgumentList "-y" -Wait -NoNewWindow

        # Add to PATH
        $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
    }

    # Update to required version
    & rustup update
    & rustup default stable

    Write-Info "Rust version: $(rustc --version)"
}

function Install-Chocolatey {
    if (!(Test-CommandExists "choco")) {
        Write-Info "Installing Chocolatey package manager..."
        Set-ExecutionPolicy Bypass -Scope Process -Force
        [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072
        Invoke-Expression ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))
    }
}

function Install-LLVM {
    Write-Info "Installing LLVM..."

    if (!(Test-CommandExists "llvm-config")) {
        Install-Chocolatey

        # Install LLVM via chocolatey
        & choco install llvm -y

        # Add to PATH
        $llvmPath = "C:\Program Files\LLVM\bin"
        if (Test-Path $llvmPath) {
            $env:PATH = "$llvmPath;$env:PATH"
            [Environment]::SetEnvironmentVariable("PATH", $env:PATH, [System.EnvironmentVariableTarget]::User)
        }
    }

    Write-Info "LLVM installed"
}

function Install-Z3 {
    Write-Info "Installing Z3..."

    if (!(Test-CommandExists "z3")) {
        Install-Chocolatey

        # Install Z3 via chocolatey
        & choco install z3 -y

        # Add to PATH if needed
        $z3Path = "C:\Program Files\Z3\bin"
        if (Test-Path $z3Path) {
            $env:PATH = "$z3Path;$env:PATH"
            [Environment]::SetEnvironmentVariable("PATH", $env:PATH, [System.EnvironmentVariableTarget]::User)
        }
    }

    Write-Info "Z3 installed"
}

function Install-VisualStudio {
    Write-Info "Checking for Visual Studio Build Tools..."

    # Check if Visual Studio or Build Tools are installed
    $vsPath = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" `
        -latest -property installationPath 2>$null

    if (!$vsPath) {
        Write-Warning-Custom "Visual Studio Build Tools not found"
        Write-Info "Please install Visual Studio Build Tools from:"
        Write-Info "  https://visualstudio.microsoft.com/downloads/"
        Write-Info ""
        Write-Info "Required components:"
        Write-Info "  - Desktop development with C++"
        Write-Info "  - Windows 10/11 SDK"
        Write-Info ""

        $response = Read-Host "Would you like to open the download page? (y/n)"
        if ($response -eq "y") {
            Start-Process "https://visualstudio.microsoft.com/downloads/"
        }

        throw "Visual Studio Build Tools are required"
    } else {
        Write-Info "✓ Visual Studio Build Tools found at: $vsPath"
    }
}

function Enable-LongPaths {
    Write-Info "Enabling long path support..."

    try {
        $regPath = "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem"
        $regName = "LongPathsEnabled"

        $currentValue = Get-ItemProperty -Path $regPath -Name $regName -ErrorAction SilentlyContinue

        if ($currentValue.$regName -ne 1) {
            Write-Info "Setting LongPathsEnabled registry key..."
            Set-ItemProperty -Path $regPath -Name $regName -Value 1 -Type DWord
            Write-Info "✓ Long path support enabled"
            Write-Warning-Custom "You may need to restart your computer for this to take effect"
        } else {
            Write-Info "✓ Long path support already enabled"
        }
    } catch {
        Write-Warning-Custom "Failed to enable long path support: $_"
        Write-Warning-Custom "You may need to enable it manually or run this script as Administrator"
    }
}

function Install-Git {
    if (!(Test-CommandExists "git")) {
        Write-Info "Installing Git..."
        Install-Chocolatey
        & choco install git -y

        # Refresh environment
        $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", [System.EnvironmentVariableTarget]::Machine) + ";" + [System.Environment]::GetEnvironmentVariable("PATH", [System.EnvironmentVariableTarget]::User)
    }
}

function Verify-Installation {
    Write-Info "Verifying installation..."

    $allGood = $true

    if (!(Test-CommandExists "rustc")) {
        Write-Error-Custom "Rust is not installed"
        $allGood = $false
    }

    if (!(Test-CommandExists "llvm-config")) {
        Write-Error-Custom "LLVM is not installed or not in PATH"
        $allGood = $false
    }

    if (!(Test-CommandExists "z3")) {
        Write-Error-Custom "Z3 is not installed or not in PATH"
        $allGood = $false
    }

    if (!(Test-CommandExists "git")) {
        Write-Error-Custom "Git is not installed"
        $allGood = $false
    }

    if ($allGood) {
        Write-Info "✓ All dependencies are installed"
        return $true
    } else {
        Write-Error-Custom "✗ Some dependencies are missing"
        return $false
    }
}

function Print-EnvironmentInfo {
    Write-Host ""
    Write-Info "=== Environment Information ==="

    try { Write-Host "Rust version: $(rustc --version)" } catch { Write-Host "Rust: not installed" }
    try { Write-Host "Cargo version: $(cargo --version)" } catch { Write-Host "Cargo: not installed" }
    try { Write-Host "LLVM version: $(llvm-config --version)" } catch { Write-Host "LLVM: not installed" }
    try { Write-Host "Z3 version: $(z3 --version)" } catch { Write-Host "Z3: not installed" }
    try { Write-Host "Git version: $(git --version)" } catch { Write-Host "Git: not installed" }

    Write-Host ""
}

function Main {
    Write-Info "=== Verum Setup Script (Windows) ==="
    Write-Info "This script will install all dependencies required to build Verum"
    Write-Host ""

    # Check if running as Administrator for some operations
    $isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (!$isAdmin) {
        Write-Warning-Custom "Not running as Administrator. Some features (like long path support) may not work."
        Write-Info "Consider running this script as Administrator for full functionality."
        Write-Host ""
    }

    try {
        # Enable long path support
        if ($isAdmin) {
            Enable-LongPaths
        }

        # Install dependencies
        Install-Git
        Install-Rust
        Install-VisualStudio
        Install-LLVM
        Install-Z3

        # Verify installation
        Write-Host ""
        $verified = Verify-Installation

        # Print environment info
        Print-EnvironmentInfo

        Write-Info "=== Setup Complete ==="
        Write-Info "You can now build Verum with: cargo build"
        Write-Host ""

        if (!$isAdmin) {
            Write-Warning-Custom "Note: If you encounter path length issues, run this script as Administrator to enable long path support"
        }

        if ($verified) {
            Write-Info "All dependencies successfully installed!"
        } else {
            Write-Warning-Custom "Some dependencies may not be properly installed. Please check the output above."
        }

    } catch {
        Write-Error-Custom "Setup failed: $_"
        exit 1
    }
}

Main
