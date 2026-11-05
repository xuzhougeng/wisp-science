$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'prepare_native_windows_webview2.ps1')

function Assert($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}
function Assert-Fails($Action, $Message) {
    $failed = $false
    try { & $Action } catch { $failed = $true }
    Assert $failed $Message
}

# Every download, signature lookup and process launch is mocked. No runtime is
# downloaded/installed, and no local registry or certificate store is modified.
function Save-NativeRuntimeDownload($Url, $Path) {
    Assert ($Url -eq 'https://go.microsoft.com/fwlink/p/?LinkId=2124703') 'Unexpected bootstrapper source.'
    $script:downloads++
    [IO.File]::WriteAllBytes($Path, $script:downloadBytes)
    if ($script:networkFailure) { throw 'Simulated interrupted download.' }
}
function Get-AuthenticodeSignature($LiteralPath) {
    Assert (Test-Path -LiteralPath $LiteralPath) 'Signature must check the actual downloaded file.'
    $script:signatureChecks++
    return [pscustomobject]@{ Status = $script:signatureStatus; SignerCertificate = $script:certificate }
}
function Start-Process { throw 'Build-time preparation must never execute the bootstrapper.' }

$temporary = Join-Path ([IO.Path]::GetTempPath()) ('wisp-webview2-test-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temporary | Out-Null
$destination = Join-Path $temporary 'folder with spaces/MicrosoftEdgeWebview2Setup.exe'
$previousTls = [Net.ServicePointManager]::SecurityProtocol
try {
    $script:downloads = 0
    $script:signatureChecks = 0
    $script:downloadBytes = [byte[]]@(0x4D, 0x5A, 1, 2)
    $script:signatureStatus = 'Valid'
    $script:certificate = [pscustomobject]@{ Subject = 'CN=Microsoft Corporation, O=Microsoft Corporation, C=US' }
    $script:networkFailure = $false

    Save-NativeWebView2Bootstrapper $destination
    Assert ($script:downloads -eq 1 -and $script:signatureChecks -eq 1) 'Download must be signature checked.'
    Assert ((Get-Item -LiteralPath $destination).Length -eq 4) 'Verified bootstrapper was not saved.'
    Assert ([Net.ServicePointManager]::SecurityProtocol -eq $previousTls) 'TLS setting was not restored.'
    Assert-NativeWebView2Bootstrapper $destination
    Assert ($script:downloads -eq 1 -and $script:signatureChecks -eq 2) 'Revalidation must work without a download.'

    foreach ($status in @('NotSigned', 'HashMismatch', 'NotTrusted', 'UnknownError')) {
        $script:signatureStatus = $status
        Assert-Fails { Save-NativeWebView2Bootstrapper $destination } "Accepted signature status $status."
    }
    $script:signatureStatus = 'Valid'
    foreach ($subject in @('CN=Microsoft Corporation, O=Other Company',
            'O=Microsoft Corporation Evil, CN=Fake', 'CN=Microsoft Corporation')) {
        $script:certificate = [pscustomobject]@{ Subject = $subject }
        Assert-Fails { Save-NativeWebView2Bootstrapper $destination } 'Accepted a different publisher.'
    }
    $script:certificate = $null
    Assert-Fails { Save-NativeWebView2Bootstrapper $destination } 'Accepted a missing signer certificate.'
    $script:certificate = [pscustomobject]@{ Subject = 'O=Microsoft Corporation, CN=Microsoft Corporation' }
    $script:networkFailure = $true
    Assert-Fails { Save-NativeWebView2Bootstrapper $destination } 'Accepted an interrupted download.'
    $script:networkFailure = $false

    foreach ($bytes in @([byte[]]@(), [byte[]]@(0x4D), [byte[]]@(60, 104, 116, 109, 108))) {
        $script:downloadBytes = $bytes
        Assert-Fails { Save-NativeWebView2Bootstrapper $destination } 'Accepted an empty/truncated/HTML download.'
    }
    $oversized = Join-Path $temporary 'standalone-runtime.exe'
    $stream = [IO.File]::Create($oversized)
    try { $stream.SetLength(10MB + 1) } finally { $stream.Dispose() }
    Assert-Fails { Assert-NativeWebView2Bootstrapper $oversized } 'Accepted a full runtime instead of the small bootstrapper.'
    Assert-Fails { Assert-NativeWebView2Bootstrapper (Join-Path $temporary 'absent.exe') } 'Accepted a missing bootstrapper.'
    Assert-Fails { Assert-NativeWebView2Bootstrapper $temporary } 'Accepted a directory.'

    Assert ((Get-Item -LiteralPath $destination).Length -eq 4) 'A failed download overwrote the verified bootstrapper.'
    Assert (@(Get-ChildItem -LiteralPath (Split-Path $destination -Parent) -Filter '*.download.exe').Count -eq 0) 'Temporary downloads were not cleaned up.'
    Assert ([Net.ServicePointManager]::SecurityProtocol -eq $previousTls) 'Failure changed TLS settings.'
    Write-Host 'WebView2 preparation mock tests passed (download, signature, failure cleanup, offline revalidation).'
} finally {
    $resolved = [IO.Path]::GetFullPath($temporary)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Test directory escaped temp.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
