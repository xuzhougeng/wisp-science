# Install pinned SSL.com CodeSignTool for Windows and require its RFC3161 TSA.
# The zip SHA-256 is the v1.3.2 Windows asset from SSLcom/CodeSignTool.

$ErrorActionPreference = 'Stop'

$script:EsignerCodeSignToolVersion = '1.3.2'
$script:EsignerCodeSignToolSha256 = '4afc32e8b7f79bbe1de7e4e7049aaad4e0f754357613b9bbec0e3052f06fd36b'
$script:EsignerCodeSignToolUrl = 'https://github.com/SSLcom/CodeSignTool/releases/download/v1.3.2/CodeSignTool-v1.3.2-windows.zip'

function Test-EsignerRfc3161Config([string]$PropertiesPath) {
    $found = 0
    foreach ($line in [IO.File]::ReadAllLines($PropertiesPath)) {
        if ($line.Trim() -eq 'TSA_URL=http://ts.ssl.com') { $found += 1 }
    }
    return $found -eq 1
}

function Assert-EsignerSha256([string]$FilePath, [string]$Expected) {
    $actual = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $wanted = $Expected.ToLowerInvariant()
    if ($actual -ne $wanted) {
        throw "CodeSignTool zip SHA-256 mismatch: expected $wanted, got $actual"
    }
}

function Resolve-EsignerCodeSignTool([string]$Root) {
    $properties = @(Get-ChildItem -Path $Root -Recurse -Filter 'code_sign_tool.properties' |
        Where-Object { $_.Directory.Name -eq 'conf' -and $_.Directory.FullName -notmatch '__MACOSX' })
    if ($properties.Count -ne 1) {
        throw "Expected one conf/code_sign_tool.properties, found $($properties.Count)"
    }
    $toolHome = $properties[0].Directory.Parent.FullName
    $jar = Join-Path (Join-Path $toolHome 'jar') ("code_sign_tool-" + $script:EsignerCodeSignToolVersion + ".jar")
    $java = Join-Path (Join-Path (Join-Path $toolHome 'jdk-11.0.2') 'bin') 'java.exe'
    if (-not (Test-Path -LiteralPath $jar)) { throw "Missing $jar" }
    if (-not (Test-Path -LiteralPath $java)) { throw "Missing $java" }
    if (-not (Test-EsignerRfc3161Config $properties[0].FullName)) {
        throw 'CodeSignTool TSA_URL must be http://ts.ssl.com (RFC3161)'
    }
    return [pscustomobject]@{ Home = $toolHome; Java = $java; Properties = $properties[0].FullName }
}

function Publish-EsignerToolEnv([string]$Home, [string]$Java) {
    $env:ESIGNER_CODESIGNTOOL_HOME = $Home
    $env:ESIGNER_JAVA = $Java
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_ENV)) { return }
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::AppendAllText($env:GITHUB_ENV, ("`nESIGNER_CODESIGNTOOL_HOME=$Home`nESIGNER_JAVA=$Java`n"), $utf8)
}

function Install-EsignerCodeSignTool {
    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TOOL_CACHE)) {
        throw 'RUNNER_TOOL_CACHE is not set'
    }
    $dest = Join-Path (Join-Path $env:RUNNER_TOOL_CACHE 'CodeSignTool') $script:EsignerCodeSignToolVersion
    $jar = Join-Path (Join-Path $dest 'jar') ("code_sign_tool-" + $script:EsignerCodeSignToolVersion + ".jar")
    if (-not (Test-Path -LiteralPath $jar)) {
        $zip = Join-Path ([IO.Path]::GetTempPath()) ("CodeSignTool-v" + $script:EsignerCodeSignToolVersion + "-windows.zip")
        $ProgressPreference = 'SilentlyContinue'
        Invoke-WebRequest -Uri $script:EsignerCodeSignToolUrl -OutFile $zip
        Assert-EsignerSha256 $zip $script:EsignerCodeSignToolSha256
        if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Recurse -Force }
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        & tar.exe -xf $zip -C $dest
        if ($LASTEXITCODE -ne 0) { throw "tar exited $LASTEXITCODE" }
        Remove-Item -LiteralPath $zip -Force
    }
    $tool = Resolve-EsignerCodeSignTool $dest
    Publish-EsignerToolEnv $tool.Home $tool.Java
    Write-Host ("CodeSignTool " + $script:EsignerCodeSignToolVersion + " ready")
}

if ($MyInvocation.InvocationName -ne '.') {
    Install-EsignerCodeSignTool
}
