# No network, UAC, Appx registration, or changes to installed runtimes.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ensure_native_windows_runtimes.ps1')
$signatureCheck = ${function:Assert-NativeMicrosoftSignature}
$downloadResolver = ${function:Get-NativeDotnetDownload}
$config = Get-Content (Join-Path $PSScriptRoot 'native_windows_runtimes.json') -Raw | ConvertFrom-Json
$versions = @()
$packages = @()
$downloads = [Collections.Generic.List[string]]::new()
$installs = [Collections.Generic.List[string]]::new()
$installerCode = 0
$badSignature = $false
$downloadFailure = $false
$registerRuntime = $true
$badHash = $false
$partialRuntime = $false
function Assert($Condition, $Message) { if (-not $Condition) { throw $Message } }
function Get-NativeDotnetVersions { $script:versions }
function Get-NativeAppRuntimePackages($Name) {
    if ($script:partialRuntime -and $Name -ne $script:config.windowsAppRuntimePackageName) { return @() }
    $script:packages
}
function RuntimePackage($Arch = 'X64', $Version = '6000.424.1611.0') {
    [pscustomobject]@{ Architecture=$Arch; Version=$Version; PublisherId='8wekyb3d8bbwe'; Status='Ok' }
}
function Get-NativeDotnetDownload($Config) {
    $sha = [Security.Cryptography.SHA512]::Create()
    try { $hash = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes('runtime')))).Replace('-', '') } finally { $sha.Dispose() }
    if ($script:badHash) { $hash = '0' * 128 }
    [pscustomobject]@{ url='https://builds.dotnet.microsoft.com/fake.exe'; hash=$hash }
}
function Save-NativeRuntimeDownload($Url, $Path) {
    if ($script:downloadFailure) { throw 'offline' }
    $script:downloads.Add($Url)
    [IO.File]::WriteAllText($Path, 'runtime', [Text.UTF8Encoding]::new($false))
}
function Assert-NativeMicrosoftSignature($Path) { if ($script:badSignature) { throw 'invalid signature' } }
function Invoke-NativeRuntimeInstaller($Path, $Kind) {
    $script:installs.Add($Kind)
    if ($script:registerRuntime -and $script:installerCode -in @(0, 3010)) {
        if ($Kind -eq 'dotnet') { $script:versions = @('8.0.24') }
        else { $script:packages = @(RuntimePackage) }
    }
    return $script:installerCode
}

$versions = @('9.0.1', '8.0.0-preview.1')
Assert (-not (Test-NativeDotnet $config)) 'Another major or preview runtime must not satisfy .NET 8.'
$versions = @('8.0.24')
Assert (Test-NativeDotnet $config) 'Compatible .NET runtime was missed.'
foreach ($candidate in @((RuntimePackage 'X86'), (RuntimePackage 'X64' '6000.0.0.0'))) {
    $packages = @($candidate)
    Assert (-not (Test-NativeAppRuntime $config)) 'Wrong architecture or old runtime was accepted.'
}
$packages = @(RuntimePackage)
$partialRuntime = $true
Assert (-not (Test-NativeAppRuntime $config)) 'Partial Windows App Runtime installation was accepted.'
$partialRuntime = $false
Assert ((Ensure-NativeWindowsRuntimes $config) -eq 0) 'Installed runtimes should succeed.'
Assert ($downloads.Count -eq 0 -and $installs.Count -eq 0) 'Installed runtimes triggered downloads.'

foreach ($missing in @('dotnet', 'windows-app-runtime', 'both')) {
    $versions = if ($missing -in @('dotnet', 'both')) { @() } else { @('8.0.24') }
    $packages = if ($missing -in @('windows-app-runtime', 'both')) { @() } else { @(RuntimePackage) }
    $downloads.Clear(); $installs.Clear()
    Assert ((Ensure-NativeWindowsRuntimes $config) -eq 0) 'Missing runtime installation failed.'
    $expected = if ($missing -eq 'both') { 2 } else { 1 }
    Assert ($downloads.Count -eq $expected -and $installs.Count -eq $expected) 'Downloaded an already installed runtime.'
}
foreach ($failure in @('network', 'signature', 'hash', 'installer', 'postcheck')) {
    $versions = @(); $packages = @(RuntimePackage)
    $downloads.Clear(); $installs.Clear()
    $downloadFailure = $failure -eq 'network'
    $badSignature = $failure -eq 'signature'
    $badHash = $failure -eq 'hash'
    $installerCode = if ($failure -eq 'installer') { 1602 } else { 0 }
    $registerRuntime = $failure -ne 'postcheck'
    $caught = $false
    try { Ensure-NativeWindowsRuntimes $config | Out-Null } catch { $caught = $true }
    Assert $caught "Runtime $failure should abort setup."
    if ($failure -in @('network', 'signature', 'hash')) { Assert ($installs.Count -eq 0) 'Unverified download was executed.' }
}
$downloadFailure = $false; $badSignature = $false; $badHash = $false; $registerRuntime = $true
$versions = @(); $installerCode = 3010
Assert ((Ensure-NativeWindowsRuntimes $config) -eq 3010) 'Reboot-required result was lost.'

$signature = [pscustomobject]@{ Status='Valid'; SignerCertificate=[pscustomobject]@{ Subject='CN=Microsoft Corporation, O=Microsoft Corporation, C=US' } }
function Get-AuthenticodeSignature { $script:signature }
& $signatureCheck 'fake.exe'
foreach ($invalid in @('untrusted', 'wrong-publisher')) {
    $signature.Status = if ($invalid -eq 'untrusted') { 'NotTrusted' } else { 'Valid' }
    $signature.SignerCertificate.Subject = 'CN=Other, O=Other Corporation, C=US'
    $caught = $false
    try { & $signatureCheck 'fake.exe' } catch { $caught = $true }
    Assert $caught 'Non-Microsoft or untrusted executable was accepted.'
}
$metadata = [pscustomobject]@{ 'latest-runtime'='8.0.24'; releases=@([pscustomobject]@{runtime=[pscustomobject]@{version='8.0.24';files=@([pscustomobject]@{rid='win-x64';name='dotnet-runtime-win-x64.exe';url='https://builds.dotnet.microsoft.com/runtime.exe';hash=('a' * 128)})}}) }
function Invoke-RestMethod { $script:metadata }
Assert ((& $downloadResolver $config).url -eq 'https://builds.dotnet.microsoft.com/runtime.exe') 'Microsoft metadata selection failed.'
$metadata.releases[0].runtime.files[0].url = 'https://untrusted.example/runtime.exe'
$caught = $false
try { & $downloadResolver $config } catch { $caught = $true }
Assert $caught 'Untrusted metadata URL was accepted.'
Write-Host 'Native Windows runtime bootstrap tests passed.'
