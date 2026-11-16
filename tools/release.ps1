# Выпуск новой версии: сборка, подпись и загрузка в открытый репозиторий выпусков.
# Версия берётся из Cargo.toml: подними её там и закоммить до запуска.
#
#   powershell -File tools\release.ps1 -Notes "Что нового одной строкой"
#
# Закрытый ключ лежит вне репозитория. Без него новые версии не поставятся ни у кого.
param(
  [Parameter(Mandatory = $true)][string]$Notes,
  [string]$Repo = 'gordidima-rgb/deloist-agent',
  [string]$Key = (Join-Path $env:USERPROFILE '.deloist\release-signing.key'),
  [string]$Target = 'C:\Users\Public\crew-target'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not (Test-Path $Key)) { throw "Нет ключа выпуска: $Key" }
if (git status --porcelain) { throw 'В дереве есть незакоммиченные правки: выпуск должен совпадать с коммитом.' }
# Специалисты каталога: оба языка, разделы инструкций, проверочные задачи, нет секретов и личных путей.
node tools/check_agents.mjs
if ($LASTEXITCODE -ne 0) { throw 'Проверка специалистов каталога не прошла (tools/check_agents.mjs).' }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"$' | Select-Object -First 1).Matches[0].Groups[1].Value
$tag = "v$version"
gh release view $tag --repo $Repo 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) { throw "Выпуск $tag уже есть: подними версию в Cargo.toml." }

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = $Target
# Без этого в публичный файл попадают пути сборки вместе с именем пользователя Windows.
# Флаги передаём через --config: переменная RUSTFLAGS отменила бы настройки из .cargo/config.toml.
$registry = Join-Path $env:USERPROFILE '.cargo\registry\src'
$remap = "target.x86_64-pc-windows-gnullvm.rustflags=['--remap-path-prefix=$registry=crates','--remap-path-prefix=$root=src']"
cargo build --release --bin deloist --bin deloist-setup --example release_sign --config $remap
if ($LASTEXITCODE -ne 0) { throw 'Сборка не прошла.' }

foreach ($name in 'deloist.exe', 'deloist-setup.exe') {
  $built = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes("$Target\release\$name"))
  if ($built.Contains($env:USERPROFILE) -or $built.Contains($root)) { throw "В файле $name остались локальные пути: выкладывать нельзя." }
}

$out = Join-Path $Target "release-out\$tag"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "$Target\release\deloist.exe" "$out\deloist.exe" -Force
# Файл для кнопки «Скачать» на лендинге: маленький загрузчик с постоянным именем
# (https://github.com/$Repo/releases/latest/download/Deloist-Setup.exe). Старые файлы выкладываем как прежде.
Copy-Item "$Target\release\deloist-setup.exe" "$out\Deloist-Setup.exe" -Force

$url = "https://github.com/$Repo/releases/download/$tag/deloist.exe"
& "$Target\release\examples\release_sign.exe" sign $Key "$out\deloist.exe" $version $url $Notes $out | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Подпись не прошла.' }
# Публичная история — коммит за коммитом, без private/; метка выпуска встаёт на код, из которого собран файл.
git fetch -q origin
git merge-base --is-ancestor HEAD origin/main
if ($LASTEXITCODE -ne 0) { throw 'Выпускаемый коммит ещё не в origin/main: сначала влей его.' }
& pwsh -NoProfile -File (Join-Path $root 'private\tools\sync-public.ps1') -Source HEAD -Push
if ($LASTEXITCODE -ne 0) { throw 'Публичная история не обновилась (private\tools\sync-public.ps1).' }
$publicHead = (git rev-parse refs/public/head).Trim()
gh release create $tag --repo $Repo --target $publicHead --title "Deloist $version" --notes $Notes `
  "$out\Deloist-Setup.exe" "$out\deloist.exe" "$out\latest.json" "$out\latest.json.sig"
if ($LASTEXITCODE -ne 0) { throw 'Загрузка не прошла.' }
"Готово: https://github.com/$Repo/releases/tag/$tag"
