[CmdletBinding()]
param(
    [ValidateRange(1, 3600)]
    [int]$PollIntervalSeconds = 30,
    [string]$TaskName = "Gareji Board Portfolio Daemon",
    [string]$Database,
    [switch]$SkipBuild,
    [switch]$NoStart
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

$arguments = "portfolio-daemon --poll-interval-seconds $PollIntervalSeconds"
if ($Database) {
    $databasePath = [System.IO.Path]::GetFullPath($Database)
    $arguments = "--database `"$databasePath`" $arguments"
}

$action = New-ScheduledTaskAction -Execute $executable -Argument $arguments
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet `
    -MultipleInstances IgnoreNew `
    -StartWhenAvailable `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 3 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -Hidden

$registeredTask = Register-ScheduledTask `
    -TaskName $TaskName `
    -Action $action `
    -Trigger $trigger `
    -Settings $settings `
    -Description "Runs the headless Gareji Board Portfolio scheduler while the user is logged in." `
    -Force

if (-not $NoStart) {
    Start-ScheduledTask -TaskName $TaskName
}

$registeredTask | Select-Object TaskName, State
