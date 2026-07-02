$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
Remove-Item Env:TRUNK_NO_COLOR -ErrorAction SilentlyContinue
Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue
Get-NetTCPConnection -LocalPort 1420 -ErrorAction SilentlyContinue |
    ForEach-Object { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue }
& "$PSScriptRoot\sync-vendor.ps1"
trunk serve --port 1420
