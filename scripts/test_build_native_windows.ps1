# Exercise the actual packaging script with fake compilers. No SDK or network.
$ErrorActionPreference = 'Stop'
$source = Split-Path $PSScriptRoot -Parent
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('wisp native build ' + [guid]::NewGuid())
$calls = [Collections.Generic.List[object]]::new()
$failBuild = ''
$originalConfig = $env:TAURI_CONFIG

function Assert-True($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}

function Invoke-WispTestPython {
    $calls.Add(@{ Tool = 'python'; Arguments = @($args) })
    $global:LASTEXITCODE = 0
}

function cargo {
    $toolArgs = @($args)
    $package = $toolArgs[$toolArgs.IndexOf('-p') + 1]
    $calls.Add(@{ Tool = 'cargo'; Arguments = $toolArgs; Config = $env:TAURI_CONFIG })
    if ($failBuild -eq $package) { $global:LASTEXITCODE = 37; return }
    $rustVariant = if ($toolArgs -contains '--release') { 'release' } else { 'debug' }
    $target = $toolArgs[$toolArgs.IndexOf('--target-dir') + 1]
    $folder = Join-Path $target $rustVariant
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Set-Content -LiteralPath (Join-Path $folder "$package.exe") -Value "$package $rustVariant"
    $global:LASTEXITCODE = 0
}

function dotnet {
    $toolArgs = @($args)
    $calls.Add(@{ Tool = 'dotnet'; Arguments = $toolArgs })
    if ($failBuild -eq 'dotnet') { $global:LASTEXITCODE = 37; return }
    $output = $toolArgs[$toolArgs.IndexOf('-o') + 1]
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    Set-Content -LiteralPath (Join-Path $output 'Wisp.Science.Preview.exe') -Value 'WinUI'
    $global:LASTEXITCODE = 0
}

try {
    foreach ($folder in @('scripts', 'src-tauri', 'ui', 'skills', 'python', 'r', 'browser-extension', 'seed')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $testRoot $folder) | Out-Null
    }
    Copy-Item (Join-Path $source 'scripts/build_native_windows.ps1') (Join-Path $testRoot 'scripts')
    Copy-Item (Join-Path $source 'src-tauri/tauri.native-windows.conf.json') (Join-Path $testRoot 'src-tauri')
    Set-Content (Join-Path $testRoot 'ui/native-host.html') '<html>host</html>'
    foreach ($resource in @('skills', 'python', 'r', 'browser-extension', 'seed')) {
        Set-Content (Join-Path $testRoot "$resource/resource.txt") $resource
    }
    $env:TAURI_CONFIG = 'original configuration'
    $testCrt = Join-Path $testRoot 'crt'
    New-Item -ItemType Directory -Path $testCrt | Out-Null
    foreach ($runtime in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
        Set-Content (Join-Path $testCrt $runtime) 'fake x64 CRT'
    }
    $initialLocation = (Get-Location).Path
    $buildScript = Join-Path $testRoot 'scripts/build_native_windows.ps1'
    foreach ($configuration in @('Debug', 'Release')) {
        $calls.Clear()
        if ($configuration -eq 'Debug') {
            & $buildScript -Configuration Debug -Python Invoke-WispTestPython -VCRedistDirectory $testCrt
        } else {
            # The standalone script keeps its release default for CI callers.
            & $buildScript -Python Invoke-WispTestPython -VCRedistDirectory $testCrt
        }
        $rustVariant = $configuration.ToLowerInvariant()
        $output = Join-Path $testRoot $(if ($configuration -eq 'Release') { 'target/native-windows-release' } else { 'target/native-windows' })
        $cargoCalls = @($calls | Where-Object { $_.Tool -eq 'cargo' })
        Assert-True ($cargoCalls.Count -eq 2) 'Expected exactly two Rust builds.'
        foreach ($call in $cargoCalls) {
            Assert-True (($call.Arguments -contains '--release') -eq ($configuration -eq 'Release')) 'Rust profile mismatch.'
            Assert-True ($call.Arguments -contains '--locked') 'Rust lockfile must be respected.'
            Assert-True ($call.Arguments -contains (Join-Path $testRoot 'target')) 'Rust target directory mismatch.'
        }
        $hostConfig = $cargoCalls[1].Config | ConvertFrom-Json
        Assert-True ($hostConfig.build.frontendDist -eq '../target/native-windows-host-assets') 'Expected inert native host assets.'
        $dotnetCalls = @($calls | Where-Object { $_.Tool -eq 'dotnet' })
        Assert-True ($dotnetCalls.Count -eq 1) 'Expected exactly one WinUI publish.'
        Assert-True ($dotnetCalls[0].Arguments -contains $configuration) 'WinUI profile mismatch.'
        foreach ($argument in @('win-x64', '--self-contained', '-p:WindowsAppSDKSelfContained=true', '-p:PublishTrimmed=false', '-p:PublishSingleFile=false', '-p:PublishReadyToRun=false')) {
            Assert-True ($dotnetCalls[0].Arguments -contains $argument) "Missing publish setting: $argument"
        }
        Assert-True (($dotnetCalls[0].Arguments -contains '-p:DebugType=None') -eq ($configuration -eq 'Release')) 'Release symbol policy mismatch.'
        Assert-True ((Get-Content (Join-Path $output 'wisp-service.exe') -Raw).Trim() -eq "wisp-service $rustVariant") 'Wrong service binary packaged.'
        Assert-True ((Get-Content (Join-Path $output 'settings-host/wisp-tauri.exe') -Raw).Trim() -eq "wisp-tauri $rustVariant") 'Wrong host binary packaged.'
        Assert-True (Test-Path (Join-Path $output 'Wisp.Science.Preview.exe')) 'Missing WinUI executable.'
        foreach ($runtime in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
            Assert-True (Test-Path (Join-Path $output $runtime)) "Missing app runtime: $runtime"
            Assert-True (Test-Path (Join-Path $output "settings-host/$runtime")) "Missing host runtime: $runtime"
        }
        foreach ($resource in @('skills', 'python', 'r', 'browser-extension', 'seed')) {
            Assert-True (Test-Path (Join-Path $output "settings-host/$resource/resource.txt")) "Missing $resource."
        }
        Assert-True ($env:TAURI_CONFIG -eq 'original configuration') 'TAURI_CONFIG was not restored.'
        Assert-True ((Get-Location).Path -eq $initialLocation) 'Working directory was not restored.'
    }
    Assert-True (Test-Path (Join-Path $testRoot 'target/native-windows/Wisp.Science.Preview.exe')) 'Release overwrote debug preview.'

    $releaseOutput = Join-Path $testRoot 'target/native-windows-release'
    Set-Content (Join-Path $releaseOutput 'obsolete.dll') 'stale dependency'
    Set-Content (Join-Path $releaseOutput 'old.pdb') 'stale symbols'
    & $buildScript -Python Invoke-WispTestPython -VCRedistDirectory $testCrt
    Assert-True (-not (Test-Path (Join-Path $releaseOutput 'obsolete.dll'))) 'Publish retained obsolete dependencies.'
    Assert-True (-not (Test-Path (Join-Path $releaseOutput 'old.pdb'))) 'Publish retained stale symbols.'

    $calls.Clear()
    & $buildScript -RuntimeMode Both -Python Invoke-WispTestPython -VCRedistDirectory $testCrt
    Assert-True (@($calls | Where-Object { $_.Tool -eq 'cargo' }).Count -eq 2) 'Dual publish should build Rust only once.'
    $publishes = @($calls | Where-Object { $_.Tool -eq 'dotnet' })
    Assert-True ($publishes.Count -eq 2) 'Expected two publishes for Both mode.'
    Assert-True ($publishes[0].Arguments -contains '-p:WindowsAppSDKSelfContained=true') 'Missing self-contained publish.'
    Assert-True ($publishes[1].Arguments -contains '-p:WindowsAppSDKSelfContained=false') 'Missing framework-dependent publish.'
    $thinArgs = $publishes[1].Arguments
    Assert-True ($thinArgs[$thinArgs.IndexOf('--self-contained') + 1] -eq 'false') 'Thin variant bundled .NET.'
    Assert-True (Test-Path (Join-Path $testRoot 'target/native-windows-release-framework-dependent/settings-host/wisp-tauri.exe')) 'Thin variant lost its helper.'
    Assert-True (Test-Path (Join-Path $releaseOutput 'Wisp.Science.Preview.exe')) 'Thin publish overwrote self-contained output.'

    foreach ($failure in @('wisp-service', 'wisp-tauri', 'dotnet')) {
        $calls.Clear()
        $failBuild = $failure
        $caught = $false
        try { & $buildScript -Python Invoke-WispTestPython -VCRedistDirectory $testCrt } catch { $caught = $true }
        Assert-True $caught "Build should fail when $failure fails."
        if ($failure -ne 'dotnet') {
            Assert-True (@($calls | Where-Object { $_.Tool -eq 'dotnet' }).Count -eq 0) 'Continued publishing after Rust failure.'
        }
        Assert-True ($env:TAURI_CONFIG -eq 'original configuration') 'Failure leaked TAURI_CONFIG.'
        Assert-True ((Get-Location).Path -eq $initialLocation) 'Failure leaked working directory.'
    }
    $failBuild = ''
    $calls.Clear()
    Remove-Item -LiteralPath (Join-Path $testCrt 'vcruntime140.dll')
    $caught = $false
    try { & $buildScript -Python Invoke-WispTestPython -VCRedistDirectory $testCrt } catch { $caught = $true }
    Assert-True $caught 'Missing VC runtime must stop publishing.'
    Assert-True (@($calls | Where-Object { $_.Tool -eq 'dotnet' }).Count -eq 0) 'Published without VC runtime.'
    Write-Host 'Native Windows build routing tests passed.'
} finally {
    $env:TAURI_CONFIG = $originalConfig
    Remove-Item -LiteralPath $testRoot -Recurse -Force
}
