param([switch]$Launch, [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release', [string]$Python = 'python')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    & $Python scripts/sync_native_design.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Native design assets are stale.' }
    & $Python scripts/sync_native_settings_contract.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Native settings contract is stale.' }
    cargo build --locked -p wisp-service
    if ($LASTEXITCODE -ne 0) { throw 'Rust query service build failed.' }
    $output = Join-Path $repo 'target/native-windows'
    $hostAssets = Join-Path $repo 'target/native-windows-host-assets'
    New-Item -ItemType Directory -Force -Path $hostAssets | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'ui/native-host.html') -Destination (Join-Path $hostAssets 'native-host.html')
    Copy-Item -LiteralPath (Join-Path $repo 'ui/native-host.html') -Destination (Join-Path $hostAssets 'index.html')
    $previousTauriConfig = $env:TAURI_CONFIG
    try {
        # Tauri tries URL parsing before filesystem parsing. An absolute Windows
        # drive path becomes a `c:` URL and crashes when joining native-host.html.
        # Resolve this relative path against src-tauri/tauri.conf.json instead.
        $env:TAURI_CONFIG = Get-Content -LiteralPath (Join-Path $repo 'src-tauri/tauri.native-windows.conf.json') -Raw
        cargo build --locked -p wisp-tauri --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Native settings host build failed.' }
    } finally { $env:TAURI_CONFIG = $previousTauriConfig }
    dotnet publish apps/windows/Wisp.Science.Preview -c $Configuration -p:Platform=x64 -o $output
    if ($LASTEXITCODE -ne 0) { throw 'WinUI preview build failed.' }
    Copy-Item -LiteralPath (Join-Path $repo 'target/debug/wisp-service.exe') -Destination $output
    $hostOutput = Join-Path $output 'settings-host'
    New-Item -ItemType Directory -Force -Path $hostOutput | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo 'target/debug/wisp-tauri.exe') -Destination $hostOutput
    foreach ($resource in @('skills', 'python', 'r', 'browser-extension', 'seed')) {
        # Replace only this known output resource, preventing stale removed files.
        $targetResource = [IO.Path]::GetFullPath((Join-Path $hostOutput $resource))
        if (-not $targetResource.StartsWith([IO.Path]::GetFullPath($hostOutput) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resource path escaped output.' }
        if (Test-Path -LiteralPath $targetResource) { Remove-Item -LiteralPath $targetResource -Recurse -Force }
        Copy-Item -LiteralPath (Join-Path $repo $resource) -Destination $targetResource -Recurse
    }
    Write-Host 'Settings host requires the Microsoft Edge WebView2 Evergreen Runtime installed on Windows.'
    Write-Host "Native Windows preview: $output/Wisp.Science.Preview.exe"
    if ($Launch) { Start-Process -FilePath (Join-Path $output 'Wisp.Science.Preview.exe') -WindowStyle Hidden }
} finally { Pop-Location }
