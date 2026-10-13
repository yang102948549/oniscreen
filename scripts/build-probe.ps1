$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path -Parent $PSScriptRoot
Push-Location $taskRoot
try {
    cargo build --release --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    python scripts/check-contained-binary.py target/release/screen-probe.exe
    if ($LASTEXITCODE -ne 0) { throw 'Executable safety invariants failed.' }
    & './target/release/screen-probe.exe' --self-test
    if ($LASTEXITCODE -ne 0) { throw 'Offscreen/process verification failed; distribution was not replaced.' }
    $taskOutput = Join-Path $taskRoot 'dist\ScreenProbe'
    New-Item -ItemType Directory -Force -Path $taskOutput | Out-Null
    Copy-Item -LiteralPath 'target\release\screen-probe.exe' -Destination $taskOutput
    Copy-Item -LiteralPath 'README.md' -Destination $taskOutput
    Copy-Item -LiteralPath 'docs','validation','reports' -Destination $taskOutput -Recurse -Force
    $taskBinary = Get-Item (Join-Path $taskOutput 'screen-probe.exe')
    # npm may inherit a PowerShell 7 PSModulePath while launching Windows PowerShell.
    # Use .NET directly so checksum generation does not depend on module autoloading.
    $taskHasher = [System.Security.Cryptography.SHA256]::Create()
    $taskStream = [System.IO.File]::OpenRead($taskBinary.FullName)
    try {
        $taskHash = [System.BitConverter]::ToString($taskHasher.ComputeHash($taskStream)).Replace('-', '')
    } finally {
        $taskStream.Dispose()
        $taskHasher.Dispose()
    }
    [pscustomobject]@{ Executable = $taskBinary.FullName; Bytes = $taskBinary.Length; SHA256 = $taskHash } | Format-List
} finally {
    Pop-Location
}
