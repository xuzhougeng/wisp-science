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
foreach ($name in @('ESIGNER_CREDENTIAL_ID', 'ESIGNER_USERNAME', 'ESIGNER_PASSWORD', 'ESIGNER_TOTP_SECRET', 'ESIGNER_PIN', 'ESIGNER_SIGN_LOG', 'ESIGNER_PROGRAM_NAME')) {
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
} finally {
    foreach ($name in $saved.Keys) {
        if ($null -eq $saved[$name]) { Remove-Item "Env:$name" -ErrorAction SilentlyContinue }
        else { Set-Item "Env:$name" $saved[$name] }
    }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'eSigner signing tests passed'
