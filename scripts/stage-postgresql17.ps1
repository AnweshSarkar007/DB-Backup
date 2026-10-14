# Stage the official PostgreSQL 17 client tools for a Windows installer.
# This script performs no database operation and never uses the current directory as a runtime
# discovery location. Review the vendor archive and supply its SHA256 before packaging.
[CmdletBinding()]
param(
  [string]$Version = '17.11-1',
  [string]$Sha256 = '80379b2c04d51c30225532e0ae04509899141e9957ed096fe749d7fd9df8f82f',
  [string]$Destination = (Join-Path $PSScriptRoot '..\resources\postgresql\17')
)
$ErrorActionPreference = 'Stop'
$url = "https://get.enterprisedb.com/postgresql/postgresql-$Version-windows-x64-binaries.zip"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("dbm-pg17-" + [guid]::NewGuid())
$zip = "$tmp.zip"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
  Invoke-WebRequest -Uri $url -OutFile $zip
  if ([string]::IsNullOrWhiteSpace($Sha256)) {
    throw 'Provide -Sha256 after verifying the official PostgreSQL archive checksum.'
  }
  $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash
  if ($actual -ne $Sha256.ToUpperInvariant()) { throw "SHA256 mismatch: $actual" }
  Expand-Archive -Path $zip -DestinationPath $tmp -Force
  $root = Get-ChildItem $tmp -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'bin\pg_dump.exe') } | Select-Object -First 1
  if (-not $root) { throw 'Archive does not contain a PostgreSQL bin directory.' }
  New-Item -ItemType Directory -Force -Path (Join-Path $Destination 'bin'), (Join-Path $Destination 'lib') | Out-Null
  Copy-Item (Join-Path $root.FullName 'bin\*.exe') (Join-Path $Destination 'bin') -Force
  Copy-Item (Join-Path $root.FullName 'bin\*.dll') (Join-Path $Destination 'bin') -Force
  if (Test-Path (Join-Path $root.FullName 'lib')) { Copy-Item (Join-Path $root.FullName 'lib\*') (Join-Path $Destination 'lib') -Recurse -Force }
  Write-Host "Staged PostgreSQL $Version client tools in $Destination"
} finally {
  Remove-Item $zip -Force -ErrorAction SilentlyContinue
  Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
