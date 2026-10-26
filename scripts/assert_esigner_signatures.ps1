# Fail the job unless each installer has a valid timestamped Authenticode
# signature and the sign log shows the pre-bundle executable was signed.
# Tauri restores an unsigned copy of the main exe after packing it.

param(
    [string[]]$Path,
    [string[]]$RequireSignedName
)

$ErrorActionPreference = 'Stop'

function Get-EsignerSignedPaths([string]$LogPath) {
    $paths = @()
    if (-not (Test-Path -LiteralPath $LogPath)) { return ,@() }
    foreach ($line in [IO.File]::ReadAllLines($LogPath)) {
        if ($line.StartsWith("signed`t")) {
            $parts = $line.Split("`t")
            if ($parts.Length -ge 2 -and -not [string]::IsNullOrWhiteSpace($parts[1])) {
                $paths += $parts[1]
            }
        }
    }
    return ,$paths
}

function Test-EsignerLogHasName([string]$LogPath, [string]$Name) {
    $suffix = $Name.Replace('/', '\').TrimStart('\')
    foreach ($signed in (Get-EsignerSignedPaths $LogPath)) {
        $normalized = $signed.Replace('/', '\')
        if ($normalized.EndsWith('\' + $suffix, [StringComparison]::OrdinalIgnoreCase)) { return $true }
        if ($normalized.Equals($suffix, [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

function Add-EsignerSummary([string]$Line) {
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) { return }
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::AppendAllText($env:GITHUB_STEP_SUMMARY, ($Line + "`n"), $utf8)
}

function Assert-EsignerSignatures {
    param([string[]]$Files, [string[]]$RequiredNames)
    if (-not $Files -or $Files.Count -eq 0) { throw 'No files to verify' }
    $log = $env:ESIGNER_SIGN_LOG
    if ([string]::IsNullOrWhiteSpace($log)) { throw 'ESIGNER_SIGN_LOG is not set' }
    if (-not (Test-Path -LiteralPath $log)) { throw "eSigner sign log not found: $log" }

    foreach ($file in $Files) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing signed file: $file" }
        $signature = Get-AuthenticodeSignature -LiteralPath $file
        $leaf = [IO.Path]::GetFileName($file)
        if ("$($signature.Status)" -ne 'Valid' -or -not $signature.SignerCertificate) {
            throw "$leaf Authenticode status $($signature.Status)"
        }
        if (-not $signature.TimeStamperCertificate) {
            throw "$leaf has no RFC3161 timestamp"
        }
        if (-not (Test-EsignerLogHasName $log $leaf)) {
            throw "$leaf was not recorded by esigner_sign.ps1"
        }
        $subject = $signature.SignerCertificate.Subject
        Write-Host ("$leaf $($signature.Status) $subject")
        Add-EsignerSummary ("- ${leaf}: $($signature.Status); $subject")
    }
    foreach ($name in $RequiredNames) {
        if (-not (Test-EsignerLogHasName $log $name)) {
            throw "Sign log has no signed $name (the pre-bundle executable must be signed before it is packed)"
        }
        Add-EsignerSummary ("- pre-bundle signed: $name")
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        Assert-EsignerSignatures -Files $Path -RequiredNames $RequireSignedName
    } catch {
        Write-Host $_.Exception.Message
        exit 1
    }
}
