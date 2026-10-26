# Authenticode-sign one file with SSL.com eSigner CodeSignTool.
# Tauri calls this once per file (%1). Only .exe and .msi are signed.
# NSIS plugin and WiX extension DLLs are skipped: each call is a cloud
# signature, and the signed installer covers those build-tool binaries.
# eSigner burns a TOTP on every sign, so consecutive signs wait out the window.

param(
    [string]$Path,
    [string]$SignScript,
    [string]$OutFile
)

$ErrorActionPreference = 'Stop'

$script:EsignerSecretNames = @(
    'ESIGNER_CREDENTIAL_ID',
    'ESIGNER_USERNAME',
    'ESIGNER_PASSWORD',
    'ESIGNER_TOTP_SECRET',
    'ESIGNER_PIN'
)

function Get-EsignerTargetKind([string]$FilePath) {
    switch ([IO.Path]::GetExtension($FilePath).ToLowerInvariant()) {
        '.exe' { 'sign' }
        '.msi' { 'sign' }
        '.dll' { 'skip' }
        default { 'reject' }
    }
}

function Assert-EsignerSecretsPresent {
    $missing = @()
    foreach ($name in $script:EsignerSecretNames) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if ([string]::IsNullOrWhiteSpace($value)) { $missing += $name }
    }
    if ($missing.Count -gt 0) {
        throw ("Missing eSigner secret(s): " + ($missing -join ', '))
    }
}

function Hide-EsignerSecrets([string]$Text) {
    if ($null -eq $Text) { return '' }
    foreach ($name in $script:EsignerSecretNames) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrEmpty($value)) {
            $Text = $Text.Replace($value, ("***" + $name + "***"))
        }
    }
    return $Text
}

function Get-EsignerTotpDelaySeconds($LastUtc, $NowUtc, [int]$WindowSeconds = 31) {
    if ($null -eq $LastUtc) { return 0 }
    $elapsed = ($NowUtc - $LastUtc).TotalSeconds
    if ($elapsed -ge $WindowSeconds) { return 0 }
    return [int][Math]::Ceiling($WindowSeconds - $elapsed)
}

function Get-EsignerJavaArguments([string]$Jar, [string]$InputFile, [string]$OutputDir, [string]$ProgramName) {
    Assert-EsignerSecretsPresent
    $javaArgs = @(
        '-jar', $Jar,
        'sign',
        ("-username=" + $env:ESIGNER_USERNAME),
        ("-password=" + $env:ESIGNER_PASSWORD),
        ("-credential_id=" + $env:ESIGNER_CREDENTIAL_ID),
        ("-totp_secret=" + $env:ESIGNER_TOTP_SECRET),
        ("-input_file_path=" + $InputFile),
        ("-output_dir_path=" + $OutputDir),
        '-malware_block=false'
    )
    if (-not [string]::IsNullOrWhiteSpace($ProgramName)) {
        $javaArgs += ("-program_name=" + $ProgramName)
    }
    return $javaArgs
}

function Write-EsignerTauriConfig([string]$SignScriptPath, [string]$ConfigPath) {
    $resolved = (Resolve-Path -LiteralPath $SignScriptPath).Path
    $config = [ordered]@{
        bundle = [ordered]@{
            windows = [ordered]@{
                signCommand = [ordered]@{
                    cmd = 'pwsh.exe'
                    args = @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $resolved, '%1')
                }
            }
        }
    }
    $json = $config | ConvertTo-Json -Depth 8
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText($ConfigPath, $json, $utf8)
}

function Get-EsignerSignLogPath {
    if (-not [string]::IsNullOrWhiteSpace($env:ESIGNER_SIGN_LOG)) { return $env:ESIGNER_SIGN_LOG }
    $temp = $env:RUNNER_TEMP
    if ([string]::IsNullOrWhiteSpace($temp)) { $temp = [IO.Path]::GetTempPath() }
    return (Join-Path $temp 'esigner-signed.log')
}

function Add-EsignerSignLog([string]$Kind, [string]$FilePath, [string]$Thumbprint) {
    $log = Get-EsignerSignLogPath
    $directory = Split-Path -Parent $log
    if (-not [string]::IsNullOrWhiteSpace($directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $line = $Kind + "`t" + $FilePath
    if (-not [string]::IsNullOrWhiteSpace($Thumbprint)) { $line = $line + "`t" + $Thumbprint }
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::AppendAllText($log, $line + "`n", $utf8)
}

function Wait-EsignerTotpWindow {
    $log = Get-EsignerSignLogPath
    $state = Join-Path (Split-Path -Parent $log) 'esigner-totp-last.txt'
    $last = $null
    if (Test-Path -LiteralPath $state) {
        $parsed = [DateTime]::MinValue
        if ([DateTime]::TryParse([IO.File]::ReadAllText($state).Trim(), [ref]$parsed)) {
            $last = $parsed.ToUniversalTime()
        }
    }
    $delay = Get-EsignerTotpDelaySeconds $last ([DateTime]::UtcNow)
    if ($delay -gt 0) {
        Write-Host ("Waiting $delay s for a fresh eSigner TOTP")
        Start-Sleep -Seconds $delay
    }
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText($state, ([DateTime]::UtcNow.ToString('o')), $utf8)
}

function Invoke-EsignerSign([string]$FilePath) {
    if ([string]::IsNullOrWhiteSpace($FilePath)) { throw 'Usage: esigner_sign.ps1 FILE' }
    if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
        throw "eSigner target does not exist: $FilePath"
    }
    $kind = Get-EsignerTargetKind $FilePath
    if ($kind -eq 'skip') {
        Write-Host ("eSigner skip " + $FilePath)
        Add-EsignerSignLog 'skipped' $FilePath ''
        return
    }
    if ($kind -ne 'sign') {
        throw "Refusing to sign unsupported file type: $FilePath"
    }

    Assert-EsignerSecretsPresent
    if ([string]::IsNullOrWhiteSpace($env:ESIGNER_CODESIGNTOOL_HOME) -or [string]::IsNullOrWhiteSpace($env:ESIGNER_JAVA)) {
        throw 'CodeSignTool is not installed. Run scripts/install_esigner_codesigntool.ps1 first.'
    }
    $jar = Get-ChildItem -Path (Join-Path $env:ESIGNER_CODESIGNTOOL_HOME 'jar') -Filter 'code_sign_tool-*.jar' | Select-Object -First 1
    if (-not $jar) { throw 'CodeSignTool jar not found' }

    $program = $env:ESIGNER_PROGRAM_NAME
    if ([string]::IsNullOrWhiteSpace($program)) { $program = 'wisp-science' }
    $outputDir = Join-Path ([IO.Path]::GetTempPath()) ('esigner-' + [guid]::NewGuid().ToString('n'))
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
    $previousNative = $PSNativeCommandUseErrorActionPreference
    $PSNativeCommandUseErrorActionPreference = $false
    try {
        Wait-EsignerTotpWindow
        $javaArgs = Get-EsignerJavaArguments $jar.FullName $FilePath $outputDir $program
        Push-Location $env:ESIGNER_CODESIGNTOOL_HOME
        try {
            $output = & $env:ESIGNER_JAVA @javaArgs 2>&1 | Out-String
            $exit = $LASTEXITCODE
        } finally {
            Pop-Location
        }
        $output = Hide-EsignerSecrets $output
        Write-Host $output
        $failed = $exit -ne 0
        foreach ($marker in @('Error', 'Exception', 'Missing required option', 'Unmatched argument')) {
            if ($output.Contains($marker)) { $failed = $true }
        }
        if ($failed) { throw "CodeSignTool failed for $FilePath (exit $exit)" }

        $leaf = [IO.Path]::GetFileName($FilePath)
        $signed = @(Get-ChildItem -LiteralPath $outputDir -Recurse -File | Where-Object { $_.Name -eq $leaf })
        if ($signed.Count -ne 1) {
            throw "Expected one signed $leaf, found $($signed.Count)"
        }
        Move-Item -LiteralPath $signed[0].FullName -Destination $FilePath -Force
        $signature = Get-AuthenticodeSignature -LiteralPath $FilePath
        if ("$($signature.Status)" -ne 'Valid') {
            throw "$leaf Authenticode status $($signature.Status)"
        }
        if (-not $signature.TimeStamperCertificate) {
            throw "$leaf has no RFC3161 timestamp"
        }
        $thumbprint = $signature.SignerCertificate.Thumbprint
        Add-EsignerSignLog 'signed' $FilePath $thumbprint
        Write-Host ("Signed $leaf $($signature.Status) $($signature.SignerCertificate.Subject)")
    } finally {
        $PSNativeCommandUseErrorActionPreference = $previousNative
        if (Test-Path -LiteralPath $outputDir) { Remove-Item -LiteralPath $outputDir -Recurse -Force }
        $logs = Join-Path $env:ESIGNER_CODESIGNTOOL_HOME 'logs'
        if (Test-Path -LiteralPath $logs) { Remove-Item -LiteralPath $logs -Recurse -Force }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        if (-not [string]::IsNullOrWhiteSpace($OutFile)) {
            if ([string]::IsNullOrWhiteSpace($SignScript)) { $SignScript = $PSCommandPath }
            Write-EsignerTauriConfig $SignScript $OutFile
        } else {
            Invoke-EsignerSign $Path
        }
    } catch {
        Write-Host $_.Exception.Message
        exit 1
    }
}
