# Authenticode-sign one file with SSL.com eSigner CodeSignTool.
# Tauri calls this once per file (%1). Only .exe and .msi are signed.
# NSIS plugin and WiX extension DLLs are skipped: each call is a cloud
# signature, and the signed installer covers those build-tool binaries.
# eSigner burns a TOTP on every sign, so consecutive signs wait out the window.
#
# CodeSignTool v1.3.2 main() discards picocli's exit code, so java -jar
# exits 0 on auth errors, invalid OTPs, and bad arguments. Success requires
# exit 0, a signed file, and "Code signed successfully:". Stdout, stderr,
# and logs/code_signing_tool.log are printed sanitized on failure.

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
$script:EsignerCodeSignTimeoutSeconds = 600
$script:EsignerOtpAttemptLimit = 4
$script:EsignerOtpRetrySeconds = 35
# Tests substitute a fake java. Production leaves this null.
$script:EsignerProcessOverride = $null

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
    $pairs = @()
    foreach ($name in $script:EsignerSecretNames) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrEmpty($value) -and $value.Length -ge 4) {
            $pairs += [pscustomobject]@{ Name = $name; Value = $value }
        }
    }
    foreach ($pair in @($pairs | Sort-Object { $_.Value.Length } -Descending)) {
        $Text = $Text.Replace([string]$pair.Value, ('***' + $pair.Name + '***'))
    }
    return $Text
}

function Get-EsignerTotpDelaySeconds($LastUtc, $NowUtc, [int]$WindowSeconds = 31) {
    if ($null -eq $LastUtc) { return 0 }
    $elapsed = ($NowUtc - $LastUtc).TotalSeconds
    if ($elapsed -ge $WindowSeconds) { return 0 }
    return [int][Math]::Ceiling($WindowSeconds - $elapsed)
}

function Get-EsignerSignSlot {
    $token = [Environment]::GetEnvironmentVariable('WISP_SIGN_SLOT')
    if ([string]::IsNullOrWhiteSpace($token)) { return 0 }
    switch -Regex ($token.Trim().ToLowerInvariant()) {
        '^(0|windows|server|tauri)$' { return 0 }
        '^(1|winui|preview|desktop)$' { return 1 }
        default {
            $sum = 0
            foreach ($ch in $token.ToCharArray()) { $sum += [int]$ch }
            return ($sum % 2)
        }
    }
}

function Get-EsignerOtpRetryDelaySeconds([int]$BaseSeconds, [int]$Slot) {
    if ($BaseSeconds -le 0) { return 0 }
    if ($Slot -lt 0) { $Slot = 0 }
    # 35s leaves the current 30s TOTP window. Slot 1 waits one extra window
    # so Windows Release and WinUI Preview retry with different codes.
    return ($BaseSeconds + ($Slot * 30))
}

function Get-EsignerStartupDelaySeconds($LastUtc, $NowUtc, [int]$Slot, [int]$WindowSeconds = 31) {
    $delay = Get-EsignerTotpDelaySeconds $LastUtc $NowUtc $WindowSeconds
    if ($null -eq $LastUtc) {
        if ($Slot -lt 0) { $Slot = 0 }
        $delay += ($Slot * 30)
    }
    return $delay
}

function Get-EsignerCodeSignDisposition([int]$ExitCode, [string]$Output, [bool]$SignedFilePresent) {
    $text = [string]$Output
    if ([regex]::IsMatch($text, '(?i)invalid\s+otp') -or [regex]::IsMatch($text, '(?i)otp\s+is\s+invalid')) {
        return 'retry-otp'
    }
    $hasError = [regex]::IsMatch($text, '(?im)(^|\r?\n)\s*Error\s*:') -or [regex]::IsMatch($text, '(?i)(\bException\b|java\.lang\.)')
    $hasSuccess = [regex]::IsMatch($text, '(?i)Code signed successfully\s*:')
    if ($ExitCode -eq 0 -and $SignedFilePresent -and $hasSuccess -and -not $hasError) {
        return 'success'
    }
    return 'fail'
}

function Format-EsignerCapturedOutput($Result, [string]$LogText) {
    $parts = New-Object System.Collections.Generic.List[string]
    if ($Result.TimedOut) { $parts.Add('CodeSignTool timed out.') }
    if (-not [string]::IsNullOrWhiteSpace($Result.StdOut)) {
        $parts.Add("stdout:`n$($Result.StdOut.TrimEnd())")
    }
    if (-not [string]::IsNullOrWhiteSpace($Result.StdErr)) {
        $parts.Add("stderr:`n$($Result.StdErr.TrimEnd())")
    }
    if (-not [string]::IsNullOrWhiteSpace($LogText)) {
        $parts.Add("log:`n$($LogText.TrimEnd())")
    }
    if ($parts.Count -eq 0) { return '(no stdout, stderr, or log)' }
    return ($parts -join "`n")
}

function Format-EsignerCodeSignFailure([string]$Leaf, $Result, [string]$LogText, [bool]$SignedFilePresent) {
    $reason = if ($Result.TimedOut) {
        'timed out'
    } elseif (-not $SignedFilePresent) {
        'did not write a signed copy'
    } else {
        'rejected the signed result'
    }
    $body = Format-EsignerCapturedOutput $Result $LogText
    return "CodeSignTool failed to sign $Leaf ($reason, exit $($Result.ExitCode)).`n$body"
}

function ConvertTo-EsignerProcessArgument([string]$Argument) {
    if ($null -eq $Argument) { return '""' }
    if ($Argument -notmatch '[\s"]') { return $Argument }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($ch in $Argument.ToCharArray()) {
        if ($ch -eq '\') {
            $slashes++
            continue
        }
        if ($ch -eq '"') {
            [void]$builder.Append('\', (($slashes * 2) + 1))
            [void]$builder.Append('"')
            $slashes = 0
            continue
        }
        if ($slashes -gt 0) {
            [void]$builder.Append('\', $slashes)
            $slashes = 0
        }
        [void]$builder.Append($ch)
    }
    if ($slashes -gt 0) { [void]$builder.Append('\', ($slashes * 2)) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-EsignerCapturedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FileName,
        [string[]]$ArgumentList = @(),
        [string]$WorkingDirectory,
        [int]$TimeoutSeconds = 600
    )
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $FileName
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardInput = $true
    $psi.CreateNoWindow = $true
    $utf8 = New-Object System.Text.UTF8Encoding $false
    $stdoutEncoding = $psi.GetType().GetProperty('StandardOutputEncoding')
    $stderrEncoding = $psi.GetType().GetProperty('StandardErrorEncoding')
    if ($null -ne $stdoutEncoding) { $stdoutEncoding.SetValue($psi, $utf8, $null) }
    if ($null -ne $stderrEncoding) { $stderrEncoding.SetValue($psi, $utf8, $null) }
    if ($WorkingDirectory) { $psi.WorkingDirectory = $WorkingDirectory }
    $argumentListProp = $psi.GetType().GetProperty('ArgumentList')
    if ($null -ne $argumentListProp) {
        $list = $argumentListProp.GetValue($psi, $null)
        foreach ($arg in $ArgumentList) { [void]$list.Add([string]$arg) }
    } else {
        $escaped = @()
        foreach ($arg in $ArgumentList) { $escaped += (ConvertTo-EsignerProcessArgument ([string]$arg)) }
        $psi.Arguments = ($escaped -join ' ')
    }

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $psi
    [void]$process.Start()
    $process.StandardInput.Close()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $timedOut = -not $process.WaitForExit($TimeoutSeconds * 1000)
    if ($timedOut) {
        try { $process.Kill() } catch {}
        try { $process.WaitForExit() } catch {}
    } else {
        # WaitForExit(int) can return before async readers finish.
        $process.WaitForExit()
    }
    $stdout = $stdoutTask.GetAwaiter().GetResult()
    $stderr = $stderrTask.GetAwaiter().GetResult()
    $code = if ($timedOut) { 124 } else { $process.ExitCode }
    return @{
        ExitCode = $code
        StdOut = [string]$stdout
        StdErr = [string]$stderr
        TimedOut = [bool]$timedOut
    }
}

function Invoke-EsignerToolProcess {
    param(
        [string]$FileName,
        [string[]]$ArgumentList,
        [string]$WorkingDirectory,
        [int]$TimeoutSeconds = 600
    )
    if ($null -ne $script:EsignerProcessOverride) {
        # Named binding keeps ArgumentList intact. Positional calls unroll that array.
        return (& $script:EsignerProcessOverride -FileName $FileName -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory -TimeoutSeconds $TimeoutSeconds)
    }
    return (Invoke-EsignerCapturedProcess -FileName $FileName -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory -TimeoutSeconds $TimeoutSeconds)
}

function Read-EsignerCodeSignLog([string]$ToolHome) {
    $logPath = Join-Path (Join-Path $ToolHome 'logs') 'code_signing_tool.log'
    if (-not (Test-Path -LiteralPath $logPath)) { return '' }
    try { $text = [IO.File]::ReadAllText($logPath) } catch { return '' }
    if ($text.Length -gt 16384) {
        $text = "(log truncated to last 16384 characters)`n" + $text.Substring($text.Length - 16384)
    }
    return $text
}

function Clear-EsignerCodeSignLog([string]$ToolHome) {
    $logDir = Join-Path $ToolHome 'logs'
    if (Test-Path -LiteralPath $logDir) {
        Remove-Item -LiteralPath $logDir -Recurse -Force -ErrorAction SilentlyContinue
    }
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

function Get-EsignerTotpStatePath {
    return (Join-Path (Split-Path -Parent (Get-EsignerSignLogPath)) 'esigner-totp-last.txt')
}

function Set-EsignerTotpTimestamp {
    $state = Get-EsignerTotpStatePath
    $directory = Split-Path -Parent $state
    if (-not [string]::IsNullOrWhiteSpace($directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText($state, ([DateTime]::UtcNow.ToString('o')), $utf8)
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
    $state = Get-EsignerTotpStatePath
    $last = $null
    if (Test-Path -LiteralPath $state) {
        $parsed = [DateTime]::MinValue
        if ([DateTime]::TryParse([IO.File]::ReadAllText($state).Trim(), [ref]$parsed)) {
            $last = $parsed.ToUniversalTime()
        }
    }
    $delay = Get-EsignerStartupDelaySeconds $last ([DateTime]::UtcNow) (Get-EsignerSignSlot)
    if ($delay -gt 0) {
        Write-Host ("Waiting $delay s for a fresh eSigner TOTP")
        Start-Sleep -Seconds $delay
    }
    Set-EsignerTotpTimestamp
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
    $leaf = [IO.Path]::GetFileName($FilePath)
    $otpSlot = Get-EsignerSignSlot
    $attemptLimit = $script:EsignerOtpAttemptLimit
    $succeeded = $false
    try {
        for ($attempt = 1; $attempt -le $attemptLimit; $attempt++) {
            $outputDir = Join-Path ([IO.Path]::GetTempPath()) ('esigner-' + [guid]::NewGuid().ToString('n'))
            New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
            try {
                if ($attempt -eq 1) {
                    Wait-EsignerTotpWindow
                } else {
                    Set-EsignerTotpTimestamp
                }
                Clear-EsignerCodeSignLog $env:ESIGNER_CODESIGNTOOL_HOME
                $javaArgs = Get-EsignerJavaArguments $jar.FullName $FilePath $outputDir $program
                $captured = Invoke-EsignerToolProcess $env:ESIGNER_JAVA $javaArgs $env:ESIGNER_CODESIGNTOOL_HOME $script:EsignerCodeSignTimeoutSeconds
                $exitCode = 0
                if ($null -ne $captured.ExitCode) { $exitCode = [int]$captured.ExitCode }
                $result = @{
                    ExitCode = $exitCode
                    StdOut = Hide-EsignerSecrets ([string]$captured.StdOut)
                    StdErr = Hide-EsignerSecrets ([string]$captured.StdErr)
                    TimedOut = [bool]$captured.TimedOut
                }
                $logText = Hide-EsignerSecrets (Read-EsignerCodeSignLog $env:ESIGNER_CODESIGNTOOL_HOME)
                $streamText = (@($result.StdOut, $result.StdErr) -join "`n")
                $signed = @(Get-ChildItem -LiteralPath $outputDir -Recurse -File -ErrorAction SilentlyContinue |
                    Where-Object { $_.Name -eq $leaf })
                $signedPresent = ($signed.Count -eq 1)
                $disposition = Get-EsignerCodeSignDisposition -ExitCode $result.ExitCode -Output $streamText -SignedFilePresent $signedPresent
                if ($result.TimedOut) { $disposition = 'fail' }
                if ($disposition -eq 'success') {
                    Move-Item -LiteralPath $signed[0].FullName -Destination $FilePath -Force
                    $succeeded = $true
                    break
                }

                $failure = Hide-EsignerSecrets (Format-EsignerCodeSignFailure -Leaf $leaf -Result $result -LogText $logText -SignedFilePresent $signedPresent)
                if ($disposition -eq 'retry-otp' -and $attempt -lt $attemptLimit) {
                    $delay = Get-EsignerOtpRetryDelaySeconds -BaseSeconds $script:EsignerOtpRetrySeconds -Slot $otpSlot
                    Write-Host ("eSigner rejected the one-time password (attempt $attempt/$attemptLimit). Waiting ${delay}s for the next TOTP window.")
                    Write-Host $failure
                    if ($delay -gt 0) { Start-Sleep -Seconds $delay }
                    continue
                }
                throw $failure
            } finally {
                if (Test-Path -LiteralPath $outputDir) {
                    Remove-Item -LiteralPath $outputDir -Recurse -Force
                }
            }
        }
        if (-not $succeeded) {
            throw (Hide-EsignerSecrets "CodeSignTool failed to sign $leaf.")
        }
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
        Clear-EsignerCodeSignLog $env:ESIGNER_CODESIGNTOOL_HOME
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
        $message = [string]$_.Exception.Message
        if ([string]::IsNullOrWhiteSpace($message)) { $message = [string]$_.ToString() }
        Write-Host (Hide-EsignerSecrets $message)
        exit 1
    }
}
