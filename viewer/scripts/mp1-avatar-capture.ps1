param(
    [switch]$SkipBuild,
    [int]$Port = 38118
)

$ErrorActionPreference = 'Stop'
$processPath = [Environment]::GetEnvironmentVariable('Path', 'Process')
if ($processPath) {
    [Environment]::SetEnvironmentVariable('PATH', $null, 'Process')
    [Environment]::SetEnvironmentVariable('Path', $processPath, 'Process')
}
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$viewer = Join-Path $repo 'viewer'
$evidence = Join-Path $viewer 'interchange\mp1-avatar-capture'
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
        if ($LASTEXITCODE -ne 0) { throw 'server/client-sim build failed' }
        cargo build --release --features multiplayer --bin triangulum-viewer
        if ($LASTEXITCODE -ne 0) { throw 'multiplayer viewer build failed' }
    }
} finally {
    Pop-Location
}

$serverExe = Join-Path $viewer 'target\release\triangulum-server.exe'
$simExe = Join-Path $viewer 'target\release\triangulum-client-sim.exe'
$viewerExe = Join-Path $viewer 'target\release\triangulum-viewer.exe'
$token = 'mp1-avatar-shot-token'
$url = "triangulum://127.0.0.1:$Port/#$token"
$shot = Join-Path $viewer 'interchange\mp1-two-clients.png'
$server = $null
$sim = $null

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

try {
    $serverOut = Join-Path $evidence 'server.out.log'
    $server = Start-Process -FilePath $serverExe -WorkingDirectory $repo -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $serverOut `
        -RedirectStandardError (Join-Path $evidence 'server.err.log') `
        -ArgumentList @(
            '--bind', "127.0.0.1:$Port", '--token', $token,
            '--assets', (Join-Path $viewer 'assets'),
            '--journal', (Join-Path $evidence 'world.edj2'), '--no-console'
        )
    Wait-ForServer $serverOut
    $sim = Start-Process -FilePath $simExe -WorkingDirectory $repo -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $evidence 'bob.out.log') `
        -RedirectStandardError (Join-Path $evidence 'bob.err.log') `
        -ArgumentList @(
            '--url', $url, '--assets', (Join-Path $viewer 'assets'), '--name', 'Bob',
            '--duration-ms', '120000', '--body', 'neisor',
            '--lat', '10.0000529', '--lon', '30.0000000', '--alt', '0.675987',
            '--yaw', '180', '--pitch', '0', '--roll', '0', '--mode', 'walk'
        )
    Start-Sleep -Milliseconds 500
    $viewerArgs = @(
        '--capture', $shot, '--join', $url, '--name', 'Alice', '--multiplayer-wait', '20',
        '--lat', '10', '--lon', '30', '--alt', '0.004', '--yaw', '0', '--pitch', '-8',
        '--size', '1280x720', '--patch', '0.7'
    )
    & $viewerExe @viewerArgs
    if ($LASTEXITCODE -ne 0) { throw "multiplayer offscreen capture failed with $LASTEXITCODE" }
    if (-not (Test-Path -LiteralPath $shot) -or (Get-Item -LiteralPath $shot).Length -lt 10000) {
        throw 'capture PNG is missing or implausibly small'
    }
    Write-Host "MP1 avatar capture: $shot"
} finally {
    if ($sim -and -not $sim.HasExited) { Stop-Process -Id $sim.Id -Force }
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
}
