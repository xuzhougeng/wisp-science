# Policy tests for eSigner signing. No CodeSignTool, network, or real secrets.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'esigner_sign.ps1')
. (Join-Path $PSScriptRoot 'install_esigner_codesigntool.ps1')
. (Join-Path $PSScriptRoot 'assert_esigner_signatures.ps1')

function Assert-True($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Throws($Script, $Expected) {
    $threw = $false
    $message = ''
    try { & $Script } catch { $threw = $true; $message = $_.Exception.Message }
    Assert-True $threw 'Expected a failure'
    Assert-True ($message.Contains($Expected)) ("Expected '$Expected' in '$message'")
    return $message
}

$saved = @{}
foreach ($name in @('ESIGNER_CREDENTIAL_ID', 'ESIGNER_USERNAME', 'ESIGNER_PASSWORD', 'ESIGNER_TOTP_SECRET', 'ESIGNER_PIN', 'ESIGNER_SIGN_LOG', 'ESIGNER_PROGRAM_NAME', 'ESIGNER_CODESIGNTOOL_HOME', 'ESIGNER_JAVA', 'WISP_SIGN_SLOT', 'FAKE_TOOL_ARGV', 'FAKE_TOOL_MODE')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
    Remove-Item "Env:$name" -ErrorAction SilentlyContinue
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("esigner-test-" + [guid]::NewGuid().ToString('n'))
New-Item -ItemType Directory -Force -Path $root | Out-Null
try {
    Assert-True ((Get-EsignerTargetKind 'app.EXE') -eq 'sign') 'exe is signed'
    Assert-True ((Get-EsignerTargetKind 'setup.msi') -eq 'sign') 'msi is signed'
    Assert-True ((Get-EsignerTargetKind 'plugin.dll') -eq 'skip') 'dll is skipped'
    Assert-True ((Get-EsignerTargetKind 'notes.txt') -eq 'reject') 'other types are rejected'

    $now = [DateTime]::Parse('2026-10-26T00:00:31Z').ToUniversalTime()
    Assert-True ((Get-EsignerTotpDelaySeconds $null $now) -eq 0) 'first sign does not wait'
    Assert-True ((Get-EsignerTotpDelaySeconds $now.AddSeconds(-10) $now) -eq 21) 'totp window is 31 seconds'
    Assert-True ((Get-EsignerTotpDelaySeconds $now.AddSeconds(-31) $now) -eq 0) 'elapsed window does not wait'
    Assert-True ((Get-EsignerTotpDelaySeconds $now $now) -eq 31) 'same timestamp waits a full window'

    $dll = Join-Path $root 'NSISdl.dll'
    Set-Content -LiteralPath $dll -Value 'dll' -Encoding ascii
    $env:ESIGNER_SIGN_LOG = Join-Path $root 'sign.log'
    Invoke-EsignerSign -FilePath $dll
    $skipLine = [IO.File]::ReadAllText($env:ESIGNER_SIGN_LOG)
    Assert-True ($skipLine.StartsWith("skipped`t")) 'dll skip is logged'
    Assert-True ($skipLine.Contains('NSISdl.dll')) 'dll name is logged'
    Assert-True (-not (Test-EsignerLogHasName $env:ESIGNER_SIGN_LOG 'NSISdl.dll')) 'skipped dll is not a signed path'

    $message = Assert-Throws { Invoke-EsignerSign -FilePath (Join-Path $root 'missing.exe') } 'does not exist'
    Assert-True (-not $message.Contains('ESIGNER_PASSWORD')) 'missing file does not talk about secrets'

    $env:ESIGNER_PASSWORD = 'super-secret-canary'
    $message = Assert-Throws { Assert-EsignerSecretsPresent } 'ESIGNER_CREDENTIAL_ID'
    Assert-True ($message.Contains('ESIGNER_PIN')) 'pin secret is required'
    Assert-True (-not $message.Contains('super-secret-canary')) 'secret values are not in the error'

    $env:ESIGNER_CREDENTIAL_ID = 'cred-canary'
    $env:ESIGNER_USERNAME = 'user-canary'
    $env:ESIGNER_TOTP_SECRET = 'totp-canary'
    $env:ESIGNER_PIN = 'pin-canary-1234'
    $javaArgs = @(Get-EsignerJavaArguments 'tool.jar' 'C:\app.exe' 'C:\out' 'wisp-science')
    $joined = $javaArgs -join "`n"
    Assert-True ($joined.Contains('-credential_id=cred-canary')) 'credential id is passed'
    Assert-True ($joined.Contains('-totp_secret=totp-canary')) 'totp secret is passed'
    Assert-True ($joined.Contains('-password=super-secret-canary')) 'password is passed'
    Assert-True ($joined.Contains('-malware_block=false')) 'malware scan override is explicit'
    Assert-True ($joined.Contains('-program_name=wisp-science')) 'program name is passed'
    Assert-True (-not $joined.Contains('pin-canary-1234')) 'pin is not sent to CodeSignTool'
    Assert-True (-not ($javaArgs | Where-Object { $_.StartsWith('-pin') })) 'no pin flag'
    Assert-True ((Hide-EsignerSecrets $joined) -eq ($joined.Replace('super-secret-canary', '***ESIGNER_PASSWORD***').Replace('cred-canary', '***ESIGNER_CREDENTIAL_ID***').Replace('user-canary', '***ESIGNER_USERNAME***').Replace('totp-canary', '***ESIGNER_TOTP_SECRET***'))) 'log redaction covers every secret'
    $env:ESIGNER_PIN = '1234'
    $env:ESIGNER_PASSWORD = '12345678'
    Assert-True ((Hide-EsignerSecrets '12345678') -eq '***ESIGNER_PASSWORD***') 'longer secrets are redacted before shorter ones'
    $env:ESIGNER_PASSWORD = 'super-secret-canary'
    $env:ESIGNER_PIN = 'pin-canary-1234'

    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output 'Error: The provided authorization grant is invalid, expired, revoked' -SignedFilePresent $false) -eq 'fail') 'exit 0 plus Error is failure'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output 'Error: invalid otp' -SignedFilePresent $false) -eq 'retry-otp') 'invalid otp retries when exit code is 0'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 1 -Output 'Error: The OTP is invalid.' -SignedFilePresent $false) -eq 'retry-otp') 'OTP is invalid retries'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output 'Code signed successfully: C:\out\wisp-science.exe' -SignedFilePresent $true) -eq 'success') 'success line and file is success'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output 'Code signed successfully: C:\out\wisp-science.exe' -SignedFilePresent $false) -eq 'fail') 'success line without file is failure'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output 'java.lang.IllegalArgumentException: Illegal base64 character' -SignedFilePresent $false) -eq 'fail') 'exception text is failure'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 3 -Output 'Code signed successfully: C:\out\wisp-science.exe' -SignedFilePresent $true) -eq 'fail') 'non-zero exit is failure'
    Assert-True ((Get-EsignerCodeSignDisposition -ExitCode 0 -Output "WARNING: sun.reflect.Reflection.getCallerClass is not supported.`nCode signed successfully: C:\out\wisp-science.exe" -SignedFilePresent $true) -eq 'success') 'JVM warning does not hide success'

    Assert-True ((Get-EsignerOtpRetryDelaySeconds -BaseSeconds 35 -Slot 0) -eq 35) 'windows slot waits one TOTP window'
    Assert-True ((Get-EsignerOtpRetryDelaySeconds -BaseSeconds 35 -Slot 1) -eq 65) 'winui slot waits an extra window'
    Assert-True ((Get-EsignerOtpRetryDelaySeconds -BaseSeconds 0 -Slot 1) -eq 0) 'tests can disable the OTP wait'
    Assert-True ((Get-EsignerStartupDelaySeconds $null $now 0) -eq 0) 'slot 0 does not delay the first sign'
    Assert-True ((Get-EsignerStartupDelaySeconds $null $now 1) -eq 30) 'slot 1 delays the first sign by one window'
    Assert-True ((Get-EsignerStartupDelaySeconds $now.AddSeconds(-10) $now 1) -eq 21) 'later signs do not add the slot delay again'
    $env:WISP_SIGN_SLOT = 'winui'
    Assert-True ((Get-EsignerSignSlot) -eq 1) 'winui sign slot'
    $env:WISP_SIGN_SLOT = 'windows'
    Assert-True ((Get-EsignerSignSlot) -eq 0) 'windows sign slot'
    Remove-Item Env:WISP_SIGN_SLOT -ErrorAction SilentlyContinue
    Assert-True ((Get-EsignerSignSlot) -eq 0) 'unset sign slot stays on the first window'
    Assert-True ((ConvertTo-EsignerProcessArgument 'simple') -eq 'simple') 'plain arguments are not quoted'
    Assert-True ((ConvertTo-EsignerProcessArgument 'P!@^^ss word"12\') -eq '"P!@^^ss word\"12\\"') 'password punctuation stays one argument'

    $failure = Format-EsignerCodeSignFailure -Leaf 'wisp-science.exe' -Result @{
        ExitCode = 0
        StdOut = 'Error: authorization grant is invalid'
        StdErr = 'err-line'
        TimedOut = $false
    } -LogText 'detail: signing attempt 1' -SignedFilePresent $false
    Assert-True ($failure.Contains('exit 0') -and $failure.Contains('did not write a signed copy') -and $failure.Contains('Error: authorization grant is invalid') -and $failure.Contains('err-line') -and $failure.Contains('detail: signing attempt 1')) 'failure text includes stdout, stderr, and log'

    $exe = Join-Path (Join-Path (Join-Path $root 'target') 'release') 'wisp-science.exe'
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $exe) | Out-Null
    Set-Content -LiteralPath $exe -Value 'exe' -Encoding ascii
    Add-EsignerSignLog 'signed' $exe 'ABCDEF'
    Assert-True (Test-EsignerLogHasName $env:ESIGNER_SIGN_LOG 'wisp-science.exe') 'signed exe is found'
    Assert-True (Test-EsignerLogHasName $env:ESIGNER_SIGN_LOG 'target/release/wisp-science.exe') 'relative suffix matches the signed exe'
    $message = Assert-Throws { Assert-EsignerSignatures -Files @((Join-Path $root 'nope.msi')) -RequiredNames @() } 'Missing signed file'

    $props = Join-Path $root 'code_sign_tool.properties'
    [IO.File]::WriteAllText($props, "CLIENT_ID=example`r`nTSA_URL=http://ts.ssl.com`r`n")
    Assert-True (Test-EsignerRfc3161Config $props) 'ssl.com tsa is accepted'
    [IO.File]::WriteAllText($props, "TSA_URL=http://timestamp.example`n")
    Assert-True (-not (Test-EsignerRfc3161Config $props)) 'other tsa urls are rejected'

    $hashFile = Join-Path $root 'hash.bin'
    [IO.File]::WriteAllBytes($hashFile, [byte[]](0x61, 0x62, 0x63))
    Assert-EsignerSha256 $hashFile 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
    $message = Assert-Throws { Assert-EsignerSha256 $hashFile '0000000000000000000000000000000000000000000000000000000000000000' } 'SHA-256 mismatch'

    $tool = Join-Path $root 'tool'
    $javaDir = Join-Path (Join-Path $tool 'jdk-11.0.2') 'bin'
    $macConf = Join-Path (Join-Path $tool '__MACOSX') 'conf'
    New-Item -ItemType Directory -Force -Path (Join-Path $tool 'conf'), (Join-Path $tool 'jar'), $javaDir, $macConf | Out-Null
    [IO.File]::WriteAllText((Join-Path (Join-Path $tool 'conf') 'code_sign_tool.properties'), "TSA_URL=http://ts.ssl.com`n")
    [IO.File]::WriteAllText((Join-Path $macConf 'code_sign_tool.properties'), 'not-the-real-file')
    Set-Content -LiteralPath (Join-Path (Join-Path $tool 'jar') 'code_sign_tool-1.3.2.jar') -Value 'jar' -Encoding ascii
    Set-Content -LiteralPath (Join-Path $javaDir 'java.exe') -Value 'java' -Encoding ascii
    $resolved = Resolve-EsignerCodeSignTool $tool
    Assert-True ([IO.Path]::GetFullPath($resolved.Home) -eq [IO.Path]::GetFullPath($tool)) 'tool home ignores __MACOSX'
    Assert-True ($resolved.Java.EndsWith('java.exe')) 'bundled java is selected'

    $config = Join-Path $root 'tauri.json'
    Write-EsignerTauriConfig -SignScriptPath $PSCommandPath -ConfigPath $config
    $parsed = Get-Content -LiteralPath $config -Raw | ConvertFrom-Json
    $signArgs = @($parsed.bundle.windows.signCommand.args)
    Assert-True ($parsed.bundle.windows.signCommand.cmd -eq 'pwsh.exe') 'tauri uses pwsh'
    Assert-True ($signArgs -contains '%1') 'tauri placeholder is preserved'
    Assert-True ($signArgs -contains (Resolve-Path -LiteralPath $PSCommandPath).Path) 'sign script path is absolute'

    $toolHome = Join-Path $root 'codesigntool'
    New-Item -ItemType Directory -Force -Path (Join-Path $toolHome 'jar') | Out-Null
    Set-Content -LiteralPath (Join-Path (Join-Path $toolHome 'jar') 'code_sign_tool-1.3.2.jar') -Value 'jar' -Encoding ascii
    $env:ESIGNER_CODESIGNTOOL_HOME = $toolHome
    $env:ESIGNER_JAVA = 'java'
    $env:ESIGNER_SIGN_LOG = Join-Path $root 'attempt.log'
    $payload = Join-Path $root 'payload.exe'
    Set-Content -LiteralPath $payload -Value 'unsigned' -Encoding ascii
    $script:EsignerOtpRetrySeconds = 0
    $state = Join-Path $root 'esigner-totp-last.txt'

    $tracker = @{ Attempts = 0; Args = @() }
    $script:EsignerProcessOverride = {
        param($FileName, $ArgumentList, $WorkingDirectory, $TimeoutSeconds)
        $tracker.Attempts++
        $tracker.Args = @($ArgumentList)
        $logDir = Join-Path $env:ESIGNER_CODESIGNTOOL_HOME 'logs'
        New-Item -ItemType Directory -Force -Path $logDir | Out-Null
        [IO.File]::WriteAllText((Join-Path $logDir 'code_signing_tool.log'), "detail: signing attempt $($tracker.Attempts)`nCredential $($env:ESIGNER_PASSWORD)")
        return @{ ExitCode = 0; StdOut = "Error: authorization grant is invalid for $($env:ESIGNER_PASSWORD)"; StdErr = 'err-line'; TimedOut = $false }
    }.GetNewClosure()
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Force }
    $message = Assert-Throws { Invoke-EsignerSign -FilePath $payload } 'Error: authorization grant is invalid'
    Assert-True ($tracker.Attempts -eq 1) 'auth failure does not retry'
    Assert-True ($message.Contains('exit 0') -and $message.Contains('did not write a signed copy') -and $message.Contains('err-line') -and $message.Contains('detail: signing attempt')) 'exit 0 auth failure includes CodeSignTool output'
    Assert-True (-not $message.Contains('super-secret-canary')) 'auth failure output hides secrets'
    Assert-True (($tracker.Args -join "`n").Contains('-password=super-secret-canary')) 'password is passed as one argument'
    Assert-True (-not (($tracker.Args | Where-Object { $_.StartsWith('-pin') }))) 'pin is not sent to CodeSignTool'
    Assert-True (@($tracker.Args) -contains '-malware_block=false') 'malware scan override is still passed'

    $otpTracker = @{ Attempts = 0 }
    $script:EsignerProcessOverride = {
        param($FileName, $ArgumentList, $WorkingDirectory, $TimeoutSeconds)
        $otpTracker.Attempts++
        return @{ ExitCode = 0; StdOut = 'Error: invalid otp'; StdErr = ''; TimedOut = $false }
    }.GetNewClosure()
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Force }
    $message = Assert-Throws { Invoke-EsignerSign -FilePath $payload } 'Error: invalid otp'
    Assert-True ($otpTracker.Attempts -eq 4) 'invalid otp retries four times when the tool exits 0'
    Assert-True (-not $message.Contains('super-secret-canary')) 'otp failure output hides secrets'

    $missingTracker = @{ Attempts = 0 }
    $script:EsignerProcessOverride = {
        param($FileName, $ArgumentList, $WorkingDirectory, $TimeoutSeconds)
        $missingTracker.Attempts++
        return @{ ExitCode = 0; StdOut = 'Code signed successfully: C:\out\payload.exe'; StdErr = ''; TimedOut = $false }
    }.GetNewClosure()
    if (Test-Path -LiteralPath $state) { Remove-Item -LiteralPath $state -Force }
    $message = Assert-Throws { Invoke-EsignerSign -FilePath $payload } 'did not write a signed copy'
    Assert-True ($missingTracker.Attempts -eq 1) 'success text without a signed file does not retry'
    Assert-True ($message.Contains('exit 0')) 'missing signed file still reports exit 0'
    $script:EsignerProcessOverride = $null

    if ($env:OS -eq 'Windows_NT') {
        $argvPath = Join-Path $root 'argv.txt'
        $env:FAKE_TOOL_ARGV = $argvPath
        Remove-Item Env:FAKE_TOOL_MODE -ErrorAction SilentlyContinue
        $fakeExe = Join-Path $root 'fake-java.exe'
        $fakeSource = Join-Path $root 'fake-java.cs'
        [IO.File]::WriteAllText($fakeSource, @'
using System;
using System.IO;
public class FakeEsignerJava {
    public static int Main(string[] args) {
        string mode = Environment.GetEnvironmentVariable("FAKE_TOOL_MODE");
        if (mode == "sleep") {
            System.Threading.Thread.Sleep(30000);
            return 0;
        }
        string argvPath = Environment.GetEnvironmentVariable("FAKE_TOOL_ARGV");
        if (!string.IsNullOrEmpty(argvPath)) {
            File.WriteAllText(argvPath, string.Join("\n", args));
        }
        Console.Out.WriteLine("out-line");
        Console.Error.WriteLine("err-line");
        return 7;
    }
}
'@)
        $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
        if (-not (Test-Path -LiteralPath $csc)) {
            $csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework\v4.0.30319\csc.exe'
        }
        if (-not (Test-Path -LiteralPath $csc)) { throw 'csc.exe was not found' }
        & $csc /nologo /target:exe "/out:$fakeExe" $fakeSource
        if ($LASTEXITCODE -ne 0) { throw "csc exited $LASTEXITCODE" }
        $secretArg = 'P!@^^ss word"12\'
        $capture = Invoke-EsignerCapturedProcess -FileName $fakeExe -ArgumentList @($secretArg) -TimeoutSeconds 30
        Assert-True ($capture.ExitCode -eq 7) 'captured process keeps the native exit code'
        Assert-True ($capture.StdOut.Contains('out-line') -and $capture.StdErr.Contains('err-line')) 'captured process keeps stdout and stderr'
        Assert-True ((Get-Content -LiteralPath $argvPath -Raw).TrimEnd() -eq $secretArg) 'process arguments keep password punctuation'
        $env:FAKE_TOOL_MODE = 'sleep'
        $timeout = Invoke-EsignerCapturedProcess -FileName $fakeExe -ArgumentList @('unused') -TimeoutSeconds 2
        Assert-True ($timeout.TimedOut -and $timeout.ExitCode -eq 124) 'process timeout is reported'
    }
} finally {
    foreach ($name in $saved.Keys) {
        if ($null -eq $saved[$name]) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
        else { Set-Item "Env:$name" $saved[$name] }
    }
    $script:EsignerProcessOverride = $null
    $script:EsignerOtpRetrySeconds = 35
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}

Write-Host 'eSigner signing tests passed'
