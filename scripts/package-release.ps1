# Builds the release binaries and packs only what users need into dist/.
# Used by the GitHub workflows and runnable locally.
# The GitHub release title must END with the short commit hash printed here
# (the in-app updater compares it), e.g. "MINT - 1.2.0 - 2026-11-02 - 56a329a".
$ErrorActionPreference = "Stop"
$commit = (git rev-parse --short=7 HEAD).Trim()
if (git status --porcelain) { Write-Warning "Uncommitted changes: the build will not match commit $commit." }
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
New-Item -ItemType Directory -Force dist | Out-Null
$stage = "dist/MINT-$commit"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory $stage | Out-Null
Copy-Item target/release/mint-gui.exe, target/release/mint-cli.exe, LICENSE, THIRD_PARTY_NOTICES.md, README.md $stage
$zip = "dist/MINT-windows-x64-$commit.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path "$stage/*" -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $(Split-Path $zip -Leaf)" | Set-Content "$zip.sha256" -NoNewline
Write-Host "Built $zip ($hash)"
