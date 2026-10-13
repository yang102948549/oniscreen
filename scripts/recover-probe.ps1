$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
$taskExecutables = @(
    (Join-Path $taskRoot 'target\guarded\debug\screen-probe.exe'),
    (Join-Path $taskRoot 'target\debug\screen-probe.exe'),
    (Join-Path $taskRoot 'target\release\screen-probe.exe'),
    (Join-Path $taskRoot 'dist\ScreenProbe\screen-probe.exe')
)
# Stop only binaries belonging to this workspace, never a same-named unrelated app.
Get-Process -Name 'screen-probe' -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -and $taskExecutables -contains $_.Path
} | Stop-Process -Force
Write-Output 'Workspace probe processes stopped. Cursor APIs were not called; visual cursor recovery is not guaranteed.'
