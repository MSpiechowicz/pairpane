# Pairpane installer for Windows.
#
#   powershell -ExecutionPolicy Bypass -c "irm https://raw.githubusercontent.com/MSpiechowicz/pairpane/main/install.ps1 | iex"
#
# Runs the release's NSIS installer silently: a per-user install in %LOCALAPPDATA%\Pairpane with a Start menu
# shortcut and an entry under Settings > Apps, the same install the in-app updater updates.
#
# Environment overrides:
#   PAIRPANE_VERSION  release tag to install (default: latest), e.g. v0.2.0
$ErrorActionPreference = "Stop"

$Repo = "MSpiechowicz/pairpane"
$AppName = "Pairpane"
$Version = if ($env:PAIRPANE_VERSION) { $env:PAIRPANE_VERSION } else { "latest" }

if (-not [Environment]::Is64BitOperatingSystem) { throw "$AppName requires 64-bit Windows." }
if ($Version -ne "latest" -and $Version -cnotmatch '^v[0-9]+\.[0-9]+\.[0-9]+\z') { throw "PAIRPANE_VERSION must be a release tag like v0.2.0 (got '$Version')." }
$Base = if ($Version -eq "latest") { "https://github.com/$Repo/releases/latest/download" } else { "https://github.com/$Repo/releases/download/$Version" }

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  # Every request in this script goes to https://github.com/<repo>/releases/...; GitHub redirects downloads to its
  # own HTTPS object storage.

  # The release's updater manifest carries each bundle's download URL.
  # GitHub serves release assets as application/octet-stream, so parse the JSON explicitly.
  $ManifestFile = Join-Path $Tmp "latest.json"
  try {
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/latest.json" -OutFile $ManifestFile
  } catch {
    throw "No release found at $Base (has a release been published yet?)"
  }
  $Manifest = Get-Content -Raw -Path $ManifestFile | ConvertFrom-Json
  $Url = [string]$Manifest.platforms.'windows-x86_64-nsis'.url
  if (-not $Url) { throw "This release has no Windows installer." }
  $Tag = [string]$Manifest.version
  if ($Tag -cnotmatch '^[0-9]+\.[0-9]+\.[0-9]+\z') { throw "latest.json announces an invalid version." }
  if ($Version -ne "latest" -and "v$Tag" -cne $Version) { throw "latest.json of $Version announces version $Tag." }
  # Exactly the Windows installer of that release in this repository, nothing else.
  $Expected = "https://github.com/$Repo/releases/download/v$Tag/Pairpane_${Tag}_x64-setup.exe"
  if (-not [string]::Equals($Url, $Expected, [StringComparison]::Ordinal)) { throw "latest.json points at an unexpected download." }
  # Only this repository's release downloads, over HTTPS.
  $Asset = [IO.Path]::GetFileName(([Uri]$Url).AbsolutePath)
  $Setup = Join-Path $Tmp $Asset

  Write-Host "==> Downloading $Asset ($($Manifest.version))" -ForegroundColor Cyan
  Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Setup

  # Nothing is installed without a checksum to compare against.
  try {
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/checksums.txt" -OutFile (Join-Path $Tmp "checksums.txt")
  } catch {
    $code = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { 0 }
    if ($code -eq 404) { throw "This release has no checksums.txt, so $Asset can't be verified. Nothing was installed." }
    if ($code -in 502, 503, 504) { throw "GitHub is temporarily unavailable (HTTP $code) while fetching checksums.txt; try again later, see https://www.githubstatus.com. Nothing was installed." }
    if ($code -eq 429) { throw "GitHub is rate limiting requests (HTTP 429 Too Many Requests) while fetching checksums.txt; try again later. Nothing was installed." }
    if ($code) { throw "Could not download checksums.txt (HTTP $code), so $Asset can't be verified. Nothing was installed." }
    throw "Could not reach $Base/checksums.txt (network error), so $Asset can't be verified. Nothing was installed."
  }
  $line = Get-Content (Join-Path $Tmp "checksums.txt") | Where-Object { $_ -match "\s$([regex]::Escape($Asset))$" } | Select-Object -First 1
  if (-not $line) { throw "checksums.txt has no entry for $Asset" }
  $expected = ($line -split "\s+")[0].ToLower()
  $actual = (Get-FileHash -Algorithm SHA256 $Setup).Hash.ToLower()
  if ($expected -ne $actual) { throw "Checksum mismatch for $Asset" }
  Write-Host "==> Checksum verified" -ForegroundColor Cyan

  # Stop a running instance so its files can be replaced.
  Get-Process -Name "pairpane" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

  Write-Host "==> Installing" -ForegroundColor Cyan
  $Process = Start-Process -FilePath $Setup -ArgumentList "/S" -Wait -PassThru
  if ($Process.ExitCode -ne 0) { throw "The installer exited with code $($Process.ExitCode)." }

  Write-Host ""
  Write-Host "Installed $AppName $($Manifest.version). Open it from the Start menu. The app updates itself."
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
