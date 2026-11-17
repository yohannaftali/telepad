# Build the portable Windows release zip.
#   pwsh scripts/package-windows.ps1            # build dist/telepad-<version>-windows-x86_64.zip (+ .sha256)
#   pwsh scripts/package-windows.ps1 -Publish   # also create GitHub release v<version> with both files
# Publishing uses the GitHub CLI (`gh auth login` once). The tag must not exist
# yet; it is created on the current commit, which must already be pushed.
# The .github/workflows/release.yml workflow does the same on a pushed v* tag.
param(
    [switch]$Publish,
    [string]$Notes = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = "telepad-$version-windows-x86_64"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage 'licenses') | Out-Null

Copy-Item target/release/telepad.exe, target/release/telepad-check.exe $stage
Copy-Item LICENSE-MIT, LICENSE-APACHE (Join-Path $stage 'licenses')
Copy-Item crates/telepad-gui/assets/fonts/LICENSE-Phosphor.txt (Join-Path $stage 'licenses/Phosphor-MIT.txt')
Copy-Item crates/telepad-gui/assets/os/ATTRIBUTION.md (Join-Path $stage 'licenses/OS-logos.md')

@'
# Example OpenSSH client config. Rename to "config" next to telepad.exe
# before the first start to import these hosts, or use Hosts > Import in the app.

# Description shown under the host name
Host example
  HostName 192.0.2.10
  User admin
  Port 22
  IdentityFile ~/.ssh/id_ed25519
'@ | Set-Content -Encoding utf8 (Join-Path $stage 'config.example')

@"
Telepad $version - portable (Windows x86_64)
=============================================

Run telepad.exe. Nothing is installed; delete the folder to remove it.

First start
  * If a file named "config" (OpenSSH client config) is next to the exe, its hosts
    are imported into telepad.db automatically.
  * Otherwise the host list starts empty: use Hosts > Add host, or Hosts > Import
    (OpenSSH config or a Telepad JSON export).

Files created next to the exe
  telepad.db           host database (SQLite) - back it up / Export to move it
  telepad.ssh_config   generated for ssh, rewritten on every change (do not edit)

Updates
  Telepad checks https://github.com/yohannaftali/telepad/releases at start and
  every 6 h. "Update now" downloads the new zip, checks its SHA-256 and replaces
  both exes; your telepad.db and config are kept. Turn it off in
  Hosts > Software update.

Requirements
  * Windows 10/11 with the built-in OpenSSH client (C:\Windows\System32\OpenSSH\ssh.exe)
  * Your SSH keys as referenced by IdentityFile; password logins are not supported
  * Windows Terminal recommended for "Open SSH" (falls back to a console window)

Command line
  telepad-check.exe [--db path] [--json] [alias ...]   one-off check, text or JSON
  telepad-check.exe --check-update                      ask GitHub for a newer release
  telepad.exe --db path                                 use another database

Telepad only runs read-only commands on monitored hosts.
"@ | Set-Content -Encoding utf8 (Join-Path $stage 'README.txt')

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
"$hash  $name.zip" | Set-Content -Encoding ascii "$zip.sha256"

Write-Output "zip:    $zip"
Write-Output "sha256: $hash"
Write-Output ("size:   {0:N1} MB" -f ((Get-Item $zip).Length / 1MB))

if ($Publish) {
    $tag = "v$version"
    $notesArgs = if ($Notes) { @('--notes', $Notes) } else { @('--generate-notes') }
    gh release create $tag $zip "$zip.sha256" --title "Telepad $version" --target (git rev-parse HEAD) @notesArgs
    if ($LASTEXITCODE -ne 0) { throw "gh release create $tag failed" }
    Write-Output "published: https://github.com/yohannaftali/telepad/releases/tag/$tag"
}
