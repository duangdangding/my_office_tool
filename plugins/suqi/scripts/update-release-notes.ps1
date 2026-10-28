$env:GH_TOKEN = ("protocol=https`nhost=github.com`n`n" | git credential fill 2>$null | Where-Object { $_ -like "password=*" }) -replace "^password=", ""
if (-not $env:GH_TOKEN) { Write-Error "no token"; exit 1 }
$env:HTTPS_PROXY = "http://127.0.0.1:7897"
node scripts/update-release-notes.mjs $args[0] $args[1]
