param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot

function Import-MsvcEnv {
    if (Get-Command link.exe -ErrorAction SilentlyContinue) {
        return
    }

    $vcvars = $null
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $installPath = & $vswhere -latest -products * `
            -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
            -property installationPath 2>$null
        if ($installPath) {
            $candidate = Join-Path $installPath "VC\Auxiliary\Build\vcvars64.bat"
            if (Test-Path $candidate) {
                $vcvars = $candidate
            }
        }
    }

    if (-not $vcvars) {
        $fallback = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
        if (Test-Path $fallback) {
            $vcvars = $fallback
        }
    }

    if (-not $vcvars) {
        Write-Warning "link.exe was not found on PATH and Visual Studio C++ Build Tools (vcvars64.bat) could not be located. Continuing with current toolchain."
        return
    }

    Write-Host "Loading MSVC environment from $vcvars..."
    $envLines = cmd.exe /c "`"$vcvars`" >nul && set"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to initialize MSVC environment via $vcvars (exit code $LASTEXITCODE)."
    }

    foreach ($line in $envLines) {
        $eq = $line.IndexOf('=')
        if ($eq -gt 0) {
            $name = $line.Substring(0, $eq)
            $value = $line.Substring($eq + 1)
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }
}

function Find-Iscc {
    $fromPath = Get-Command iscc -ErrorAction SilentlyContinue
    if ($fromPath) {
        return $fromPath.Source
    }

    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    )

    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return $candidate
        }
    }

    throw "Inno Setup 6 (ISCC.exe) was not found. Install it with: winget install JRSoftware.InnoSetup"
}

function Get-CargoVersion {
    param([string]$CargoTomlPath)
    $content = Get-Content -Raw -Path $CargoTomlPath
    if ($content -match '(?m)^version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
    throw "Could not determine package version from $CargoTomlPath"
}

function Get-CargoTargetDir {
    $metadataJson = cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to read cargo metadata (exit code $LASTEXITCODE)."
    }
    $metadata = $metadataJson | ConvertFrom-Json
    return $metadata.target_directory
}

Push-Location $repoRoot
try {
    $version = Get-CargoVersion -CargoTomlPath (Join-Path $repoRoot "Cargo.toml")
    $iscc = Find-Iscc

    if (-not $SkipBuild) {
        Import-MsvcEnv
        Write-Host "`n==> Building release binary (v$version)"
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }

    $cargoTargetDir = Get-CargoTargetDir
    $exePath = Join-Path $cargoTargetDir "release\microphone-locker.exe"
    if (-not (Test-Path $exePath)) {
        throw "Release binary not found at $exePath. Run without -SkipBuild or run 'cargo build --release' first."
    }

    $outputDir = Join-Path $repoRoot "target\installer"
    $issPath = Join-Path $repoRoot "installer\microphone-locker.iss"
    Write-Host "`n==> Compiling Inno Setup installer (v$version)"
    & $iscc "/DAppVersion=$version" "/DSourceExe=$exePath" "/DOutputDir=$outputDir" $issPath
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }

    $installerPath = Join-Path $outputDir "MicrophoneLocker-Setup.exe"
    Write-Host "`nInstaller built successfully: $installerPath"
}
finally {
    Pop-Location
}
