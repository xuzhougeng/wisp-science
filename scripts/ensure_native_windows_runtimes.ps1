param([string]$ConfigPath)
$ErrorActionPreference = 'Stop'

function Get-NativeDotnetVersions {
    # A 32-bit installation or an SDK of another major version is insufficient.
    $location = (Get-ItemProperty -LiteralPath 'HKLM:/SOFTWARE/dotnet/Setup/InstalledVersions/x64' -Name InstallLocation -ErrorAction SilentlyContinue).InstallLocation
    if (-not $location) { $location = Join-Path $env:ProgramW6432 'dotnet' }
    $root = Join-Path $location 'shared/Microsoft.NETCore.App'
    if (Test-Path -LiteralPath $root) {
        Get-ChildItem -LiteralPath $root -Directory | ForEach-Object { $_.Name }
    }
}

function Get-NativeAppRuntimePackages($Name) {
    # Query the installing user, not packages provisioned for other accounts.
    @(Get-AppxPackage -Name $Name -ErrorAction Stop)
}

function Test-NativeDotnet($Config) {
    foreach ($installed in @(Get-NativeDotnetVersions)) {
        $version = $null
        if ([version]::TryParse($installed, [ref]$version) -and
            "$($version.Major).$($version.Minor)" -eq $Config.dotnetChannel -and
            $version -ge [version]$Config.dotnetMinimumVersion) { return $true }
    }
    return $false
}

function Test-NativeAppRuntime($Config) {
    foreach ($package in @(Get-NativeAppRuntimePackages $Config.windowsAppRuntimePackageName)) {
        if ($package.Architecture.ToString() -eq 'X64' -and
            $package.PublisherId -eq '8wekyb3d8bbwe' -and
            $package.Status -eq 'Ok' -and
            [version]$package.Version -ge [version]$Config.windowsAppRuntimeMinimumVersion) {
            # A framework left by a partial install is insufficient on Windows 10.
            $complete = $true
            foreach ($name in @($Config.windowsAppRuntimeMainPackageName,
                    $Config.windowsAppRuntimeSingletonPackageName,
                    "Microsoft.WinAppRuntime.DDLM.$($package.Version)-x6")) {
                $healthy = @(Get-NativeAppRuntimePackages $name | Where-Object {
                    $_.PublisherId -eq '8wekyb3d8bbwe' -and $_.Status -eq 'Ok' -and
                    $_.Architecture.ToString() -in @('X64', 'Neutral') -and
                    [version]$_.Version -ge [version]$Config.windowsAppRuntimeMinimumVersion
                })
                if ($healthy.Count -eq 0) { $complete = $false }
            }
            if ($complete) { return $true }
        }
    }
    return $false
}

function Get-NativeDotnetDownload($Config) {
    $url = "https://builds.dotnet.microsoft.com/dotnet/release-metadata/$($Config.dotnetChannel)/releases.json"
    $metadata = Invoke-RestMethod -Uri $url -TimeoutSec 120
    $release = @($metadata.releases | Where-Object { $_.runtime.version -eq $metadata.'latest-runtime' })[0]
    $download = @($release.runtime.files | Where-Object { $_.rid -eq 'win-x64' -and $_.name -eq 'dotnet-runtime-win-x64.exe' })
    $runtimeVersion = [version]$release.runtime.version
    if ($download.Count -ne 1 -or $runtimeVersion -lt [version]$Config.dotnetMinimumVersion -or
        "$($runtimeVersion.Major).$($runtimeVersion.Minor)" -ne $Config.dotnetChannel) { throw 'No compatible .NET 8 x64 runtime download in Microsoft metadata.' }
    $uri = [uri]$download[0].url
    if ($uri.Scheme -ne 'https' -or $uri.Host -notin @('builds.dotnet.microsoft.com', 'download.visualstudio.microsoft.com', 'dotnetcli.azureedge.net')) { throw 'Unexpected .NET download URL.' }
    if ($download[0].hash -notmatch '^[a-fA-F0-9]{128}$') { throw 'Missing .NET SHA-512.' }
    return $download[0]
}

function Save-NativeRuntimeDownload($Url, $Path) {
    Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing -TimeoutSec 300
}

function Assert-NativeMicrosoftSignature($Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
        throw 'Runtime download does not have a valid Microsoft signature.'
    }
}

function Invoke-NativeRuntimeInstaller($Path, $Kind) {
    if ($Kind -eq 'dotnet') {
        # .NET's machine-wide installer needs elevation; UAC denial is a failure.
        $process = Start-Process -FilePath $Path -ArgumentList '/install', '/quiet', '/norestart' -Verb RunAs -WindowStyle Hidden -PassThru -Wait
    } else {
        # Keep the original user identity for Windows App Runtime registration.
        $process = Start-Process -FilePath $Path -ArgumentList '--quiet' -WindowStyle Hidden -PassThru -Wait
    }
    return $process.ExitCode
}

function Ensure-NativeWindowsRuntimes($Config) {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $temporary = Join-Path ([IO.Path]::GetTempPath()) ('wisp-runtimes-' + [guid]::NewGuid())
    $reboot = $false
    try {
        foreach ($kind in @('dotnet', 'windows-app-runtime')) {
            $present = if ($kind -eq 'dotnet') { Test-NativeDotnet $Config } else { Test-NativeAppRuntime $Config }
            if ($present) { Write-Host "$kind x64 runtime already installed."; continue }
            New-Item -ItemType Directory -Force -Path $temporary | Out-Null
            $installer = Join-Path $temporary "$kind.exe"
            Write-Host "Downloading missing $kind x64 runtime from Microsoft..."
            if ($kind -eq 'dotnet') {
                $download = Get-NativeDotnetDownload $Config
                Save-NativeRuntimeDownload $download.url $installer
                if ((Get-FileHash -LiteralPath $installer -Algorithm SHA512).Hash -ne $download.hash) { throw '.NET runtime checksum mismatch.' }
            } else {
                Save-NativeRuntimeDownload $Config.windowsAppRuntimeInstallerUrl $installer
            }
            Assert-NativeMicrosoftSignature $installer
            $code = Invoke-NativeRuntimeInstaller $installer $kind
            if ($code -notin @(0, 3010)) { throw "$kind installation failed with exit code $code." }
            if ($code -eq 3010) { $reboot = $true }
            $present = if ($kind -eq 'dotnet') { Test-NativeDotnet $Config } else { Test-NativeAppRuntime $Config }
            if (-not $present) { throw "$kind still unavailable after installation. Restart if requested, then retry." }
        }
        if ($reboot) { return 3010 }
        return 0
    } finally {
        $resolved = [IO.Path]::GetFullPath($temporary)
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Runtime temporary path escaped temp.' }
        if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        if (-not [Environment]::Is64BitProcess) { throw 'Runtime setup must run in 64-bit Windows PowerShell.' }
        $config = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
        exit (Ensure-NativeWindowsRuntimes $config)
    } catch {
        Write-Host "Runtime setup failed: $($_.Exception.Message)"
        exit 1
    }
}
