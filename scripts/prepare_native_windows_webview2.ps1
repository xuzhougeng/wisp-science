param([string]$Path, [switch]$ValidateOnly)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ensure_native_windows_runtimes.ps1')

# Build-time preparation only. Never execute downloaded code on the build host.
# Microsoft documents this redistributable Evergreen bootstrapper at:
# https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution
function Assert-NativeWebView2Bootstrapper($Path) {
    $file = Get-Item -LiteralPath $Path -ErrorAction Stop
    if ($file.PSIsContainer -or $file.Length -lt 2 -or $file.Length -gt 10MB) {
        throw 'Expected the small WebView2 Evergreen bootstrapper (at most 10 MiB).'
    }
    $stream = [IO.File]::OpenRead($file.FullName)
    try {
        if ($stream.ReadByte() -ne 0x4D -or $stream.ReadByte() -ne 0x5A) {
            throw 'WebView2 bootstrapper is not a Windows executable.'
        }
    } finally { $stream.Dispose() }
    Assert-NativeMicrosoftSignature $file.FullName
}

function Save-NativeWebView2Bootstrapper($Path) {
    $destination = [IO.Path]::GetFullPath($Path)
    $temporary = $destination + '.' + [guid]::NewGuid() + '.download.exe'
    $previousTls = [Net.ServicePointManager]::SecurityProtocol
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
        Save-NativeRuntimeDownload 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' $temporary
        Assert-NativeWebView2Bootstrapper $temporary
        Move-Item -LiteralPath $temporary -Destination $destination -Force
    } finally {
        [Net.ServicePointManager]::SecurityProtocol = $previousTls
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        if (-not $Path) { throw 'Provide -Path for the WebView2 bootstrapper.' }
        if ($ValidateOnly) { Assert-NativeWebView2Bootstrapper $Path }
        else { Save-NativeWebView2Bootstrapper $Path }
        Write-Host 'Microsoft WebView2 bootstrapper signature verified; no runtime was installed.'
        exit 0
    } catch {
        Write-Host "WebView2 bootstrapper preparation failed: $($_.Exception.Message)"
        exit 1
    }
}
