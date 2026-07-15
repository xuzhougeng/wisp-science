$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
Remove-Item Env:TRUNK_NO_COLOR -ErrorAction SilentlyContinue
Remove-Item Env:NO_COLOR -ErrorAction SilentlyContinue
$devPort = 1421
$listeners = @(Get-NetTCPConnection -LocalPort $devPort -State Listen -ErrorAction SilentlyContinue)
foreach ($listener in $listeners) {
    $owner = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
    if ($null -ne $owner -and $owner.ProcessName -eq "trunk") {
        Stop-Process -Id $owner.Id -Force
    } else {
        $ownerName = if ($null -ne $owner) { $owner.ProcessName } else { "PID $($listener.OwningProcess)" }
        throw "Development port $devPort is already used by $ownerName. Stop that process or change the configured dev port."
    }
}
& "$PSScriptRoot\sync-vendor.ps1"
& trunk serve --address 127.0.0.1 --port $devPort --dist dist-dev
exit $LASTEXITCODE
