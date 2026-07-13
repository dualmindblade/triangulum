# deploy_tunnel.ps1 - create/repair the Cloudflare named tunnel for
# triangulum.dieorwrite.net -> the local MP1 server. Idempotent: safe
# to re-run; reuses an existing tunnel and repoints DNS if needed.
#
# Requires .cloudfare-creds at the repo root (gitignored) whose API
# token carries BOTH:
#   Zone    -> DNS               -> Edit   (verified present 2026-07-13)
#   Account -> Cloudflare Tunnel -> Edit   (must be added in the dash:
#     dash.cloudflare.com -> My Profile -> API Tokens -> edit token)
#
# Output: .tunnel-token at the repo root (gitignored). Run with:
#   viewer\target\release\triangulum-server.exe --token GAMETOKEN `
#     --public-url wss://triangulum.dieorwrite.net
#   tools\cloudflared.exe tunnel run --token-file .tunnel-token

param(
    [string]$Hostname = 'triangulum.dieorwrite.net',
    [string]$Service = 'http://localhost:7777'
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$creds = Get-Content (Join-Path $root '.cloudfare-creds') | ConvertFrom-StringData
$h = @{ Authorization = "Bearer $($creds.CLOUDFARE_API_TOKEN)"; 'Content-Type' = 'application/json' }
$zone = $creds.CLOUDFARE_ZONE_ID
$api = 'https://api.cloudflare.com/client/v4'

$acct = (Invoke-RestMethod -Uri "$api/zones/$zone" -Headers $h).result.account.id

# 1. Named tunnel (reuse when it already exists).
$found = (Invoke-RestMethod -Uri "$api/accounts/$acct/cfd_tunnel?name=triangulum&is_deleted=false" -Headers $h).result
if ($found.Count -gt 0) {
    $tid = $found[0].id
    Write-Host "tunnel exists: $tid"
} else {
    $bytes = New-Object byte[] 32
    (New-Object System.Security.Cryptography.RNGCryptoServiceProvider).GetBytes($bytes)
    $body = @{ name = 'triangulum'; tunnel_secret = [Convert]::ToBase64String($bytes); config_src = 'cloudflare' } | ConvertTo-Json
    $tid = (Invoke-RestMethod -Method Post -Uri "$api/accounts/$acct/cfd_tunnel" -Headers $h -Body $body).result.id
    Write-Host "tunnel created: $tid"
}

# 2. Ingress: the game hostname to the local server, everything else 404.
$cfg = @{ config = @{ ingress = @(
    @{ hostname = $Hostname; service = $Service },
    @{ service = 'http_status:404' }
) } } | ConvertTo-Json -Depth 6
Invoke-RestMethod -Method Put -Uri "$api/accounts/$acct/cfd_tunnel/$tid/configurations" -Headers $h -Body $cfg | Out-Null
Write-Host "ingress: $Hostname -> $Service"

# 3. Proxied CNAME to the tunnel (create or repoint).
$sub = $Hostname.Split('.')[0]
$target = "$tid.cfargotunnel.com"
$existing = (Invoke-RestMethod -Uri "$api/zones/$zone/dns_records?type=CNAME&name=$Hostname" -Headers $h).result
$rec = @{ type = 'CNAME'; name = $sub; content = $target; proxied = $true } | ConvertTo-Json
if ($existing.Count -gt 0) {
    Invoke-RestMethod -Method Put -Uri "$api/zones/$zone/dns_records/$($existing[0].id)" -Headers $h -Body $rec | Out-Null
    Write-Host "DNS updated: $Hostname -> $target"
} else {
    Invoke-RestMethod -Method Post -Uri "$api/zones/$zone/dns_records" -Headers $h -Body $rec | Out-Null
    Write-Host "DNS created: $Hostname -> $target"
}

# 4. Connector token to a gitignored file, never echoed.
$tt = (Invoke-RestMethod -Uri "$api/accounts/$acct/cfd_tunnel/$tid/token" -Headers $h).result
Set-Content -Path (Join-Path $root '.tunnel-token') -Value $tt -Encoding ascii
Write-Host "connector token written to .tunnel-token (gitignored)"
Write-Host "next: tools\cloudflared.exe tunnel run --token-file .tunnel-token"
