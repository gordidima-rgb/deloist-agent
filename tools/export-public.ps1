# Export the public tree: HEAD without the private/ folder (open core).
#   pwsh -NoProfile -File tools\export-public.ps1 -Out C:\Projects\deloist-public
# Only committed files are exported (git archive), so untracked and ignored files never leak.
param(
    [Parameter(Mandatory = $true)][string]$Out
)

$ErrorActionPreference = 'Stop'
$repo = (git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
if (Test-Path $Out) {
    if (Get-ChildItem -Force $Out | Select-Object -First 1) { throw "Output folder is not empty: $Out" }
} else {
    New-Item -ItemType Directory -Path $Out | Out-Null
}
$zip = Join-Path ([System.IO.Path]::GetTempPath()) ("deloist-public-" + [guid]::NewGuid().ToString('N') + '.zip')
try {
    git -C $repo archive --format=zip -o $zip HEAD -- . ':(exclude)private'
    if ($LASTEXITCODE -ne 0) { throw 'git archive failed' }
    Expand-Archive -LiteralPath $zip -DestinationPath $Out -Force
} finally {
    Remove-Item -Force -ErrorAction SilentlyContinue $zip
}
if (Test-Path (Join-Path $Out 'private')) { throw 'private/ is present in the export' }
$count = (Get-ChildItem -Recurse -File -Force $Out | Measure-Object).Count
Write-Host "Exported $count files to $Out (private/ excluded)"
