<#
.SYNOPSIS
    Byte-for-byte parity check against the Python reference implementation.
.DESCRIPTION
    Builds the Rust binary, then runs the Rust CLI and the Python reference in
    python/ over a fixed set of argument combinations and compares stdout,
    stderr and exit codes. Only line endings and path separators are
    normalised, so a pass means both tools produced the same report text.

    Both CLIs are started through System.Diagnostics.Process with
    CreateNoWindow, so no console window ever flashes on screen, and their
    output is captured through explicit pipes rather than the console.

    Shows a single-line progress bar while the cases run, then prints the
    failures and a summary. Exits 1 when anything differs.
.PARAMETER Target
    Directory the scan roots are relative to; both CLIs run from here.
.PARAMETER Roots
    Roots handed to every case, relative to -Target or absolute.
.PARAMETER ExtraTrees
    Absolute trees scanned as whole-repository cases.
.PARAMETER Profile
    Cargo profile to build and test (default: release).
.PARAMETER SkipBuild
    Reuse the existing binary instead of running cargo build.
.PARAMETER NoProgress
    Print one line per case instead of an in-place progress bar.
.ENVIRONMENT
    PARITY_PYTHON  Interpreter used for the reference run (default: auto-detected).
.EXAMPLE
    ./scripts/check-parity.ps1
.EXAMPLE
    ./scripts/check-parity.ps1 -Target C:\work\app -Roots src,crates -SkipBuild
#>
[CmdletBinding()]
param(
    [string]$Target,
    [string[]]$Roots = @("src-tauri/src", "src-tauri/crates"),
    [string[]]$ExtraTrees = @(),
    [ValidateSet("debug", "release")]
    [string]$Profile = "release",
    [switch]$SkipBuild,
    [switch]$NoProgress
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
# Default to the sibling repository this tool was written against.
if (-not $Target) { $Target = Join-Path (Split-Path -Parent $repo) "Clipboard" }

$manifest = Join-Path $repo "Cargo.toml"
$exe = Join-Path $repo ("target/" + $Profile + "/rust-loc.exe")
$pythonSource = Join-Path $repo "python/src"

if (-not (Test-Path -LiteralPath $Target)) { throw "no such directory: $Target" }
if (-not (Test-Path -LiteralPath (Join-Path $pythonSource "rust_loc"))) {
    throw "the Python reference is missing from python/src"
}

function Resolve-Python {
    if ($env:PARITY_PYTHON -and (Test-Path -LiteralPath $env:PARITY_PYTHON)) {
        return $env:PARITY_PYTHON
    }
    $found = Get-Command python.exe -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    # Windows PowerShell 5.1 skips PATH entries that end with a backslash when
    # discovering commands, so look them up by hand as well.
    foreach ($entry in ($env:PATH -split ";")) {
        if (-not $entry) { continue }
        $candidate = Join-Path $entry.TrimEnd("\") "python.exe"
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    $launcher = Get-Command py.exe -ErrorAction SilentlyContinue
    if ($launcher) { return $launcher.Source }
    throw "cannot find python.exe; set PARITY_PYTHON to the interpreter to use"
}

function Quote-Native([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    $quotes = [char]34
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append($quotes)
    $slashes = 0
    foreach ($ch in $Value.ToCharArray()) {
        if ($ch -eq [char]92) { $slashes++; continue }
        if ($ch -eq $quotes) {
            [void]$builder.Append([char]92, $slashes * 2 + 1)
            [void]$builder.Append($quotes)
            $slashes = 0
            continue
        }
        if ($slashes -gt 0) { [void]$builder.Append([char]92, $slashes); $slashes = 0 }
        [void]$builder.Append($ch)
    }
    if ($slashes -gt 0) { [void]$builder.Append([char]92, $slashes * 2) }
    [void]$builder.Append($quotes)
    return $builder.ToString()
}

function Invoke-Tool {
    param([string]$File, [string[]]$Arguments, [hashtable]$Environment)

    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $File
    $start.Arguments = (($Arguments | ForEach-Object { Quote-Native $_ }) -join " ")
    $start.WorkingDirectory = $Target
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    $start.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $start.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    if ($Environment) {
        foreach ($key in $Environment.Keys) { $start.EnvironmentVariables[$key] = $Environment[$key] }
    }

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $start
    [void]$process.Start()
    $errors = $process.StandardError.ReadToEndAsync()
    $output = $process.StandardOutput.ReadToEnd()
    $process.WaitForExit()
    return [pscustomobject]@{
        Stdout = $output
        Stderr = $errors.Result
        Code   = $process.ExitCode
    }
}

function Read-Normalised([string]$Text) {
    $Text = $Text -replace "`r`n", "`n"
    return ($Text -replace [regex]::Escape("\"), "/")
}

function Find-FirstDifference([string]$Left, [string]$Right) {
    $leftLines = $Left -split "`n"
    $rightLines = $Right -split "`n"
    $count = [Math]::Max($leftLines.Count, $rightLines.Count)
    for ($index = 0; $index -lt $count; $index++) {
        $a = if ($index -lt $leftLines.Count) { $leftLines[$index] } else { "<missing>" }
        $b = if ($index -lt $rightLines.Count) { $rightLines[$index] } else { "<missing>" }
        if ($a -cne $b) {
            return @("    line " + ($index + 1), "      python: " + $a, "      rust:   " + $b)
        }
    }
    return @()
}

$python = Resolve-Python

# Every case is a label plus the arguments handed to both CLIs.
$cases = [ordered]@{}
$cases["text"] = @($Roots)
$cases["json"] = @("--format", "json") + $Roots
$cases["json-uloc"] = @("--format", "json", "--uloc", "--show-excluded") + $Roots
$cases["include-tests"] = @("--include-tests") + $Roots
$cases["by-file"] = @("--by-file", "--sort", "code", "--top", "20") + $Roots
$cases["by-file-complexity"] = @("--by-file", "--sort", "complexity", "--top", "15") + $Roots
$cases["by-file-path"] = @("--by-file", "--sort", "path", "--top", "10") + $Roots
$cases["by-module"] = @("--by-module", "--uloc", "--show-excluded") + $Roots
$cases["csv"] = @("--format", "csv", "--by-file", "--uloc") + $Roots
$cases["csv-total"] = @("--format", "csv") + $Roots
$cases["markdown"] = @("--format", "markdown", "--by-module", "--uloc") + $Roots
$cases["uloc"] = @("--uloc", "--show-excluded") + $Roots
$cases["exclude"] = @("--exclude", "*/tests/*", "--by-file", "--top", "8") + $Roots
$cases["no-ignore-dirs"] = @("--no-ignore-dirs", "--top", "5") + $Roots
foreach ($root in $Roots) {
    $label = "root:" + ($root -replace "[^A-Za-z0-9]", "_")
    $cases[$label] = @("--uloc", $root)
}
foreach ($tree in $ExtraTrees) {
    $label = "tree:" + ([IO.Path]::GetFileName($tree.TrimEnd("/", "\")))
    $cases[$label] = @("--format", "json", "--uloc", $tree)
}

function Write-Progress-Bar([int]$Done, [int]$Total, [string]$Label) {
    if ($NoProgress) { return }
    $width = 24
    $filled = [int][Math]::Floor($width * $Done / [Math]::Max($Total, 1))
    $bar = ("#" * $filled) + ("-" * ($width - $filled))
    $line = "[{0}] {1,3}/{2}  {3}" -f $bar, $Done, $Total, $Label
    Write-Host ($line.PadRight(64)) -NoNewline
    Write-Host ([char]13) -NoNewline
}

$passed = 0
$failures = @()
$total = $cases.Count
$done = 0

foreach ($name in $cases.Keys) {
    $arguments = $cases[$name]
    $done++
    Write-Progress-Bar $done $total $name

    $reference = Invoke-Tool $python (@("-m", "rust_loc") + $arguments) @{ PYTHONPATH = $pythonSource; PYTHONIOENCODING = "utf-8"; PYTHONDONTWRITEBYTECODE = "1" }
    $rust = Invoke-Tool $exe $arguments $null

    $differences = @()
    $expected = Read-Normalised $reference.Stdout
    $actual = Read-Normalised $rust.Stdout
    if ($expected -cne $actual) {
        $differences += "  stdout differs:"
        $differences += Find-FirstDifference $expected $actual
    }
    $expectedError = Read-Normalised $reference.Stderr
    $actualError = Read-Normalised $rust.Stderr
    if ($expectedError -cne $actualError) {
        $differences += "  stderr differs:"
        $differences += Find-FirstDifference $expectedError $actualError
    }
    if ($reference.Code -ne $rust.Code) {
        $differences += ("  exit code: python " + $reference.Code + " / rust " + $rust.Code)
    }

    if ($differences.Count -eq 0) {
        $passed++
        if ($NoProgress) { Write-Host ("PASS  " + $name) -ForegroundColor Green }
    } else {
        $failures += [pscustomobject]@{ Name = $name; Arguments = $arguments; Details = $differences }
        if ($NoProgress) { Write-Host ("FAIL  " + $name) -ForegroundColor Red }
    }
}

if (-not $NoProgress) { Write-Host "" }
foreach ($failure in $failures) {
    Write-Host ("FAIL  " + $failure.Name + "  (" + ($failure.Arguments -join " ") + ")") -ForegroundColor Red
    foreach ($line in $failure.Details) { Write-Host $line }
}
Write-Host ("parity: " + $passed + " passed, " + $failures.Count + " failed, " + $total + " cases")
if ($failures.Count -gt 0) { exit 1 }
exit 0
