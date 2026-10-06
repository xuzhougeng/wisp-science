param([switch]$Launch, [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release', [string]$Python = 'python', [string]$VCRedistDirectory = '', [ValidateSet('SelfContained', 'FrameworkDependent', 'Both')][string]$RuntimeMode = 'SelfContained')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    & $Python scripts/sync_native_design.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Native design assets are stale.' }
    & $Python scripts/sync_native_settings_contract.py --check
    if ($LASTEXITCODE -ne 0) { throw 'Native settings contract is stale.' }
    $rustArgs = @('build', '--locked', '--target-dir', (Join-Path $repo 'target'))
    $rustProfile = 'debug'
    $output = Join-Path $repo 'target/native-windows'
    if ($Configuration -eq 'Release') {
        $rustArgs += '--release'
        $rustProfile = 'release'
        $output = Join-Path $repo 'target/native-windows-release'
    }
    & cargo @rustArgs -p wisp-service
    if ($LASTEXITCODE -ne 0) { throw 'Rust query service build failed.' }
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
        & cargo @rustArgs -p wisp-tauri --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Native settings host build failed.' }
    } finally { $env:TAURI_CONFIG = $previousTauriConfig }
    # Rust's MSVC executables require the VC runtime even when .NET and WinUI
    # are self-contained. Use the redistributable DLLs, never System32 copies.
    if (-not $VCRedistDirectory) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Install MSVC Build Tools or pass -VCRedistDirectory (x64 Microsoft.VC143.CRT).' }
        $vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -ne 0 -or -not $vsRoot) { throw 'Could not locate MSVC Build Tools.' }
        $redistRoot = Join-Path $vsRoot 'VC/Redist/MSVC'
        $VCRedistDirectory = Get-ChildItem -LiteralPath $redistRoot -Directory |
            Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Join-Path $_.FullName 'x64/Microsoft.VC143.CRT' } |
            Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
    if (-not $VCRedistDirectory) { throw 'Could not locate the x64 MSVC redistributable DLLs.' }
    foreach ($runtime in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
        if (-not (Test-Path -LiteralPath (Join-Path $VCRedistDirectory $runtime))) { throw "Missing MSVC runtime: $runtime" }
    }
    $baseOutput = $output
    $modes = if ($RuntimeMode -eq 'Both') { @('SelfContained', 'FrameworkDependent') } else { @($RuntimeMode) }
    foreach ($mode in $modes) {
        $selfContained = if ($mode -eq 'SelfContained') { 'true' } else { 'false' }
        $output = if ($mode -eq 'SelfContained') { $baseOutput } else { "$baseOutput-framework-dependent" }
        # Publish into a fresh directory: dotnet publish does not remove obsolete
        # dependencies or symbols left by a previous configuration.
        $targetRoot = [IO.Path]::GetFullPath((Join-Path $repo 'target')) + [IO.Path]::DirectorySeparatorChar
        $output = [IO.Path]::GetFullPath($output)
        if (-not $output.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Publish path escaped target.' }
        if (Test-Path -LiteralPath $output) { Remove-Item -LiteralPath $output -Recurse -Force }
        $publishArgs = @('publish', 'apps/windows/Wisp.Science.Preview', '-c', $Configuration,
            '-p:Platform=x64', '-r', 'win-x64', '--self-contained', $selfContained,
            "-p:WindowsAppSDKSelfContained=$selfContained", '-p:PublishTrimmed=false',
            '-p:PublishSingleFile=false', '-p:PublishReadyToRun=false', '-o', $output)
        if ($Configuration -eq 'Release') {
            # Keep XAML/reflection support; reduce size without unsafe trimming/AOT.
            $publishArgs += @('-p:DebugType=None', '-p:DebugSymbols=false', '-p:CopyOutputSymbolsToPublishDirectory=false')
        }
        & dotnet @publishArgs
        if ($LASTEXITCODE -ne 0) { throw 'WinUI preview build failed.' }
        Copy-Item -LiteralPath (Join-Path $repo "target/$rustProfile/wisp-service.exe") -Destination $output
        $hostOutput = Join-Path $output 'settings-host'
        New-Item -ItemType Directory -Force -Path $hostOutput | Out-Null
        Copy-Item -LiteralPath (Join-Path $repo "target/$rustProfile/wisp-tauri.exe") -Destination $hostOutput
        foreach ($runtime in Get-ChildItem -LiteralPath $VCRedistDirectory -Filter '*.dll' -File) {
            Copy-Item -LiteralPath $runtime.FullName -Destination $output
            Copy-Item -LiteralPath $runtime.FullName -Destination $hostOutput
        }
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
    }
} finally { Pop-Location }
