$ErrorActionPreference = "Stop"

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
        Write-Warning "link.exe was not found on PATH and Visual Studio C++ Build Tools (vcvars64.bat) could not be located. Continuing with current toolchain; if linking fails, install Visual Studio Build Tools ('Desktop development with C++') or run from a Developer PowerShell."
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

function Invoke-Step {
    param(
        [string]$Label,
        [string[]]$Command
    )
    Write-Host "`n==> $Label"
    & $Command[0] $Command[1..($Command.Length - 1)]
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Import-MsvcEnv

Invoke-Step "Format check (cargo fmt --all -- --check)" @("cargo", "fmt", "--all", "--", "--check")
Invoke-Step "Lint (cargo clippy --all-targets -- -D warnings)" @("cargo", "clippy", "--all-targets", "--", "-D", "warnings")
Invoke-Step "Tests (cargo test)" @("cargo", "test")

Write-Host "`nAll checks passed."
