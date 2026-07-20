[CmdletBinding()]
param(
    [ValidateRange(1, 1440)]
    [int]$IntervalMinutes = 5,
    [string]$TaskName = "Gareji Board Portfolio Scheduler",
    [string]$Database,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "target\release\gareji-board.exe"

if (-not $SkipBuild) {
    cargo build --release -p gareji-board-bridge --bin gareji-board --manifest-path (Join-Path $repoRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) {
        throw "The Gareji Board scheduler executable could not be built."
    }
}
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Scheduler executable not found at $executable. Run again without -SkipBuild."
}

$arguments = "portfolio-tick-due"
if ($Database) {
    $databasePath = [System.IO.Path]::GetFullPath($Database)
    $arguments = "--database `"$databasePath`" $arguments"
}

$action = New-ScheduledTaskAction -Execute $executable -Argument $arguments
$trigger = New-ScheduledTaskTrigger `
    -Once `
    -At (Get-Date).AddMinutes(1) `
    -RepetitionInterval (New-TimeSpan -Minutes $IntervalMinutes)
$settings = New-ScheduledTaskSettingsSet `
    -MultipleInstances IgnoreNew `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Minutes 2)

Register-ScheduledTask `
    -TaskName $TaskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Description "Runs one bounded Gareji Board Portfolio scheduler pass." `
    -Force | Select-Object TaskName, State
