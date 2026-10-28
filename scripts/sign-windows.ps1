param(
  [Parameter(Mandatory = $true)]
  [string]$Path
)

$ErrorActionPreference = 'Stop'
if (-not $env:WINDOWS_CERT_PATH -or -not $env:WINDOWS_CERT_PASSWORD) {
  throw '缺少 WINDOWS_CERT_PATH 或 WINDOWS_CERT_PASSWORD'
}

$signTool = (Get-Command signtool.exe -ErrorAction SilentlyContinue).Source
if (-not $signTool) {
  $sdkBin = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
  $signTool = Get-ChildItem $sdkBin -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '[\\/]x64[\\/]signtool\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signTool) { throw '未找到 signtool.exe，请安装 Windows SDK' }

& $signTool sign /fd SHA256 /td SHA256 /tr 'http://timestamp.digicert.com' /f $env:WINDOWS_CERT_PATH /p $env:WINDOWS_CERT_PASSWORD $Path
if ($LASTEXITCODE -ne 0) { throw "代码签名失败：$Path" }
