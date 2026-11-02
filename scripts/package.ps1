param([string]$OutputDirectory = 'artifacts', [string]$BinaryPath = '')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$packageRoot = if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory } else { Join-Path $projectRoot $OutputDirectory }
$bundle = Join-Path $packageRoot 'DOTAPE-Windows-x64'
New-Item -ItemType Directory -Path $bundle -Force | Out-Null
if (!$BinaryPath) { $BinaryPath = Join-Path $projectRoot 'target\release\DOTAPE.exe' }
Copy-Item -LiteralPath $BinaryPath -Destination $bundle
foreach ($name in @('LICENSE','THIRD-PARTY-NOTICES.txt','RELINKING.txt')) {
    Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $bundle
}
$fontLicense = Join-Path $bundle 'Galmuri-OFL.txt'
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\QUICKSTART.txt') -Destination (Join-Path $bundle 'README.txt')
Copy-Item -LiteralPath (Join-Path $projectRoot 'assets\fonts\LICENSE.txt') -Destination $fontLicense
Compress-Archive -LiteralPath $bundle -DestinationPath (Join-Path $packageRoot 'DOTAPE-Windows-x64.zip') -Force
