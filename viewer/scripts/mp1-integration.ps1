param(
    [switch]$SkipBuild,
    [int]$Port = 38117
)

$ErrorActionPreference = 'Stop'
# Some automation hosts inject both `Path` and `PATH` into the Windows
# process block. Windows PowerShell's Start-Process rejects that duplicate;
# normalize it without changing the effective search path.
$processPath = [Environment]::GetEnvironmentVariable('Path', 'Process')
if ($processPath) {
    [Environment]::SetEnvironmentVariable('PATH', $null, 'Process')
    [Environment]::SetEnvironmentVariable('Path', $processPath, 'Process')
}
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$viewer = Join-Path $repo 'viewer'
$evidence = Join-Path $viewer 'interchange\mp1-integration'
if (-not $evidence.StartsWith((Join-Path $viewer 'interchange'), [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "refusing to clean unexpected evidence path: $evidence"
}
if (Test-Path -LiteralPath $evidence) {
    Remove-Item -LiteralPath $evidence -Recurse -Force
}
New-Item -ItemType Directory -Path $evidence | Out-Null

Push-Location $viewer
try {
    if (-not $SkipBuild) {
        cargo build --release -p triangulum-server --bins
        if ($LASTEXITCODE -ne 0) { throw 'headless binary build failed' }
    }
} finally {
    Pop-Location
}

$serverExe = Join-Path $viewer 'target\release\triangulum-server.exe'
$clientExe = Join-Path $viewer 'target\release\triangulum-client-sim.exe'
$journal = Join-Path $evidence 'world.edj2'
$token = 'mp1-scripted-token'
$url = "triangulum://127.0.0.1:$Port/#$token"
$serverOut = Join-Path $evidence 'server.out.log'
$serverErr = Join-Path $evidence 'server.err.log'

$serverArgs = @(
    '--bind', "127.0.0.1:$Port",
    '--token', $token,
    '--assets', (Join-Path $viewer 'assets'),
    '--journal', $journal,
    '--time', '1234.5',
    '--time-scale', '1',
    '--no-console'
)
$server = Start-Process -FilePath $serverExe -ArgumentList $serverArgs -WorkingDirectory $repo `
    -RedirectStandardOutput $serverOut -RedirectStandardError $serverErr -WindowStyle Hidden -PassThru
$alice = $null
$bob = $null
$mismatch = $null

function Wait-ForServer([string]$LogPath) {
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Test-Path -LiteralPath $LogPath) -and
            ((Get-Content -LiteralPath $LogPath -Raw -ErrorAction SilentlyContinue) -match 'TRIANGULUM SERVER READY')) {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "server did not become ready; inspect $LogPath"
}

function Start-Sim([string]$Label, [string[]]$Arguments) {
    $stdout = Join-Path $evidence "$Label.out.log"
    $stderr = Join-Path $evidence "$Label.err.log"
    $process = Start-Process -FilePath $clientExe -ArgumentList $Arguments -WorkingDirectory $repo `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr -WindowStyle Hidden -PassThru
    # Windows PowerShell 5 discards ExitCode for redirected processes unless
    # the native process handle is acquired before the child exits.
    $null = $process.Handle
    return @{ Process = $process; Out = $stdout; Err = $stderr }
}

function Wait-Sim($Client, [string]$Label, [int]$TimeoutMs = 15000) {
    if (-not $Client.Process.WaitForExit($TimeoutMs)) {
        Stop-Process -Id $Client.Process.Id -Force
        $Client.Process.WaitForExit()
        throw "$Label did not exit within $TimeoutMs ms"
    }
}

try {
    Wait-ForServer $serverOut
    $common = @('--url', $url, '--assets', (Join-Path $viewer 'assets'), '--sample-server-ms', '4000')
    $bob = Start-Sim 'bob' ($common + @(
        '--name', 'Bob', '--duration-ms', '6500',
        '--body', 'neisor', '--lat', '10.000000', '--lon', '30.000000', '--alt', '0.010000'
    ))
    Start-Sleep -Milliseconds 250
    $alice = Start-Sim 'alice' ($common + @(
        '--name', 'Alice', '--duration-ms', '6000',
        '--body', 'moon', '--lat', '12.500000', '--lon', '-33.250000', '--alt', '0.004000',
        '--yaw', '91', '--pitch', '-7', '--roll', '2', '--mode', 'walk',
        '--edit', 'neisor', '2', '12345', '54321', '7', '--edit-delay-ms', '1000'
    ))
    Wait-Sim $alice 'Alice'
    Wait-Sim $bob 'Bob'
    if ($alice.Process.ExitCode -ne 0 -or $bob.Process.ExitCode -ne 0) {
        throw "normal clients failed: Alice=$($alice.Process.ExitCode) Bob=$($bob.Process.ExitCode)"
    }

    $aliceClock = Select-String -Path $alice.Out -Pattern '^CLOCK_SAMPLE ' | Select-Object -Last 1
    $bobClock = Select-String -Path $bob.Out -Pattern '^CLOCK_SAMPLE ' | Select-Object -Last 1
    if (-not $aliceClock -or -not $bobClock) { throw 'missing post-slew clock samples' }
    $clockRegex = 'canonical_s=([0-9.]+) error_ms=([0-9.]+)'
    if ($aliceClock.Line -notmatch $clockRegex) { throw 'could not parse Alice clock sample' }
    $aliceCanonical = [double]$Matches[1]; $aliceError = [double]$Matches[2]
    if ($bobClock.Line -notmatch $clockRegex) { throw 'could not parse Bob clock sample' }
    $bobCanonical = [double]$Matches[1]; $bobError = [double]$Matches[2]
    $betweenMs = [Math]::Abs($aliceCanonical - $bobCanonical) * 1000.0
    if ($aliceError -gt 50 -or $bobError -gt 50 -or $betweenMs -gt 50) {
        throw "clock convergence failed: Alice error=$aliceError Bob error=$bobError between=$betweenMs ms"
    }
    if (-not (Select-String -Path $bob.Out -Quiet -Pattern 'EDIT_APPLIED .*body=Neisor face=2 ci=12345 cj=54321 value=7 journal_records=[1-9][0-9]*')) {
        throw 'Alice edit did not appear in Bob journal'
    }
    if (-not (Select-String -Path $bob.Out -Quiet -Pattern 'PRESENCE .*from_name=Alice body=Moon lat_deg=12.500000 lon_deg=-33.250000 alt_km=0.004000')) {
        throw 'Alice body-local moon pose did not round-trip exactly to Bob'
    }
    if (-not (Test-Path -LiteralPath $journal) -or (Get-Item -LiteralPath $journal).Length -le 4) {
        throw 'server EDJ2 journal was not persisted'
    }
    $journalBytes = [System.IO.File]::ReadAllBytes($journal)
    if ([System.Text.Encoding]::ASCII.GetString($journalBytes, 0, 4) -ne 'EDJ2' -or
        (($journalBytes.Length - 4) % 42) -ne 0) {
        throw 'persisted journal is not a complete EDJ2 file'
    }
    $persistedMatch = $false
    for ($offset = 4; $offset -lt $journalBytes.Length; $offset += 42) {
        $body = $journalBytes[$offset + 16]
        $face = $journalBytes[$offset + 17]
        $ci = [System.BitConverter]::ToUInt64($journalBytes, $offset + 18)
        $cj = [System.BitConverter]::ToUInt64($journalBytes, $offset + 26)
        $value = [System.BitConverter]::ToInt64($journalBytes, $offset + 34)
        if ($body -eq 0 -and $face -eq 2 -and $ci -eq 12345 -and $cj -eq 54321 -and $value -eq 7) {
            $persistedMatch = $true
            break
        }
    }
    if (-not $persistedMatch) { throw 'Alice edit bytes are absent from persisted EDJ2' }
    if (-not (Select-String -Path $serverOut -Quiet -Pattern 'EDIT PERSISTED .*face=2 ci=12345 cj=54321 value=7')) {
        throw 'server did not log durable edit acceptance'
    }

    $mismatch = Start-Sim 'mismatch' @(
        '--url', $url, '--assets', (Join-Path $viewer 'assets'), '--name', 'MismatchProbe',
        '--build-hash', 'definitely-not-the-server-build', '--duration-ms', '1000'
    )
    Wait-Sim $mismatch 'MismatchProbe' 5000
    if ($mismatch.Process.ExitCode -eq 0) { throw 'build mismatch client was not refused' }
    if (-not (Select-String -Path $mismatch.Err -Quiet -Pattern 'REFUSED .*build hash mismatch')) {
        throw 'client mismatch refusal was not loud and specific'
    }
    Start-Sleep -Milliseconds 200
    if (-not (Select-String -Path $serverErr -Quiet -Pattern 'REFUSED .*build hash mismatch')) {
        throw 'server mismatch refusal was not loud and specific'
    }

    $summary = @(
        'MP1 INTEGRATION PASS',
        "clock: Alice/Bob common canonical sample delta = $([Math]::Round($betweenMs, 3)) ms; client slew errors = $aliceError / $bobError ms",
        'edit: Alice value 7 observed by Bob and server EDJ2 persisted',
        'presence: Alice Moon (12.5, -33.25, 0.004 km) observed exactly by Bob',
        'identity: mismatched build refused loudly by both ends',
        "journal: $journal ($((Get-Item -LiteralPath $journal).Length) bytes)"
    )
    $summary | Set-Content -LiteralPath (Join-Path $evidence 'summary.txt')
    $summary | ForEach-Object { Write-Host $_ }
} finally {
    foreach ($client in @($alice, $bob, $mismatch)) {
        if ($client -and -not $client.Process.HasExited) {
            Stop-Process -Id $client.Process.Id -Force
            $client.Process.WaitForExit()
        }
    }
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force
        $server.WaitForExit()
    }
    $transcript = Join-Path $evidence 'transcript.txt'
    @(
        '=== SUMMARY ===',
        $(if (Test-Path (Join-Path $evidence 'summary.txt')) { Get-Content (Join-Path $evidence 'summary.txt') } else { 'INTEGRATION ABORTED' }),
        '=== SERVER STDOUT ===', $(if (Test-Path $serverOut) { Get-Content $serverOut }),
        '=== SERVER STDERR ===', $(if (Test-Path $serverErr) { Get-Content $serverErr }),
        '=== ALICE ===', $(if (Test-Path (Join-Path $evidence 'alice.out.log')) { Get-Content (Join-Path $evidence 'alice.out.log') }),
        '=== BOB ===', $(if (Test-Path (Join-Path $evidence 'bob.out.log')) { Get-Content (Join-Path $evidence 'bob.out.log') }),
        '=== MISMATCH STDERR ===', $(if (Test-Path (Join-Path $evidence 'mismatch.err.log')) { Get-Content (Join-Path $evidence 'mismatch.err.log') })
    ) | Set-Content -LiteralPath $transcript
}
