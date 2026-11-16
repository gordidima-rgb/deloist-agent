//! Где бот хозяин, а где гость.
//!
//! Своя папка (воркспейс) — делай что хочешь без вопросов. Читать можно где угодно на
//! компьютере, кроме секретов. Менять что-то вне своей папки — только с разрешения человека
//! (в режиме «спрашивать»). Секреты (ключи SSH, `secrets.json`, служебная папка программы,
//! профили браузеров) недоступны инструментам файлов ни в каком режиме.
//!
//! Разбор команды — эвристика по тексту, а не стена: стену держит песочница ОС, когда она
//! включена. Всё, что разобрать не удалось, считается «неизвестно где» и спрашивается.

use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use regex::Regex;
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9+.-]+://").unwrap());
static DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[=,;'])([a-z]:(?:[\\/].*)?)$").unwrap());
static HARMLESS_REDIRECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\d?>\s*(&\s*\d|nul\b)").unwrap());

/// Сравнимый путь: канонический, если путь существует, иначе канонический предок плюс хвост.
/// `None` — даже предка нет (например, несуществующий диск).
pub fn real_path(path: &Path) -> Option<PathBuf> {
    let path = normalize(path);
    let mut existing = path.as_path();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while std::fs::symlink_metadata(existing).is_err() {
        tail.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    }
    let mut real = std::fs::canonicalize(existing).ok()?;
    for part in tail.into_iter().rev() {
        real.push(part);
    }
    Some(real)
}

/// `.` и `..` по тексту — так же, как это делает сама Windows до обращения к диску.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn canonical_or_same(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Путь задан от корня: `C:\…`, `\…`, `\\server\…`.
pub fn is_absolute_like(raw: &str) -> bool {
    Path::new(raw).components().any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
}

/// Путь для показа модели: без служебного `\\?\`.
pub fn display(path: &Path) -> String {
    let text = path.display().to_string();
    match text.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text),
    }
}

/// Чем опасен путь, если он ведёт к секретам. `real` — из [`real_path`].
/// Причина «файл с ключами»: такой файл открывает карточка `secret` (см. `keys`), остальные секреты — нет.
pub const KEY_FILE: &str = "файл с секретами";
/// Закодированную команду не удалось проверить: отказ в любом режиме.
pub const ENCODED_UNCHECKABLE: &str = "закодированную команду не удалось проверить";
pub fn secret_reason(real: &Path, data_dir: &Path, workspace: &Path) -> Option<&'static str> {
    let data = canonical_or_same(data_dir);
    let home = canonical_or_same(workspace);
    if real.starts_with(&data) && !real.starts_with(&home) {
        return Some("служебные файлы программы (ключи и токены)");
    }
    // Папка активного файла мастер-ключа закрыта целиком и наглухо, а не только по имени файла.
    if let Some(dir) = crate::secrets::key_file_path().and_then(|p| p.parent().map(canonical_or_same)) {
        if real.starts_with(&dir) {
            return Some("ключ шифрования секретов Deloist");
        }
    }
    let names: Vec<String> =
        real.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let has = |name: &str| names.iter().any(|n| n == name);
    for dir in [".ssh", ".aws", ".gnupg", ".azure", ".kube", ".docker", ".deloist"] {
        if has(dir) {
            return Some("ключи и пароли");
        }
    }
    if let Some(file) = names.last() {
        // Мастер-ключ шифрования секретов закрыт наглухо, без карточки: это не ключ проекта.
        if file == "secrets.key" {
            return Some("ключ шифрования секретов Deloist");
        }
        if file.starts_with("secrets.json") || file == ".env" || file.starts_with(".env.") {
            return Some(KEY_FILE);
        }
    }
    // Профили браузеров: пароли, cookie, сессии кабинетов.
    if has("appdata") && (has("user data") || (has("profiles") && has("mozilla"))) {
        return Some("профиль браузера (пароли и cookie)");
    }
    // Хранилище учётных данных Windows.
    if has("appdata") && has("microsoft") && (has("credentials") || has("protect") || has("vault")) {
        return Some("хранилище паролей Windows");
    }
    None
}

// ---------------------------------------------------------------------------
// Команды
// ---------------------------------------------------------------------------

/// Только читают: им можно смотреть куда угодно, кроме секретов.
const READ_ONLY: &[&str] = &[
    "dir", "type", "tree", "more", "findstr", "find", "where", "fc", "comp", "echo", "ver", "vol", "whoami",
    "hostname", "systeminfo", "ipconfig", "tasklist", "cd", "chdir", "pushd", "popd",
];

/// Меняют систему целиком, путь им не нужен: спрашиваем всегда.
const SYSTEM_WIDE: &[&str] = &[
    "shutdown", "format", "diskpart", "bcdedit", "reg", "regedit", "schtasks", "sc", "net", "netsh",
    "taskkill", "wmic", "takeown", "icacls", "cacls", "setx", "runas", "vssadmin", "wevtutil", "cipher",
    "powercfg", "msiexec", "sfc", "dism", "mountvol", "fsutil", "label", "subst", "bitsadmin", "logoff",
];

/// Только читающие команды PowerShell (внутри `powershell -c "…"`). Всё остальное там спрашивается:
/// по конвейеру PowerShell передаёт объекты, и `gci C:\x | Remove-Item` удаляет чужое без пути в тексте.
const PS_READ_ONLY: &[&str] = &[
    "get-childitem", "gci", "ls", "dir", "get-content", "gc", "cat", "type", "select-string", "sls",
    "test-path", "get-item", "gi", "measure-object", "measure", "select-object", "select", "sort-object",
    "sort", "format-list", "fl", "format-table", "ft", "out-string", "get-filehash", "resolve-path",
    "get-location", "pwd", "write-output", "echo", "set-location", "sl", "cd",
];

const GIT_READ_ONLY: &[&str] =
    &["status", "log", "diff", "show", "ls-files", "rev-parse", "blame", "describe", "grep", "shortlog"];

/// Ключи PowerShell, после которых идёт значение.
const PS_VALUE_FLAGS: &[&str] =
    &["-ex", "-ep", "-executionpolicy", "-w", "-windowstyle", "-inputformat", "-if", "-outputformat", "-of", "-version", "-v"];

/// Нужно ли спросить человека перед этой командой (режим «спрашивать»).
#[cfg(test)]
pub fn command_needs_approval(command: &str, workspace: &Path, data_dir: &Path) -> bool {
    command_needs_approval_in(command, &Places { workspace, projects: &[], data_dir, install: None })
}

/// Где бот хозяин: своя папка и папки проектов (checkout в режиме «Разработка Deloist»),
/// служебная папка и папка запущенной программы.
pub struct Places<'a> {
    pub workspace: &'a Path,
    /// Папки проектов с теми же правами, что своя папка.
    pub projects: &'a [PathBuf],
    pub data_dir: &'a Path,
    /// Папка запущенного exe: её менять нельзя ни в каком режиме.
    pub install: Option<&'a Path>,
}

impl Places<'_> {
    fn ctx(&self) -> Ctx<'_> {
        let mut homes = vec![canonical_or_same(self.workspace)];
        homes.extend(self.projects.iter().map(|p| canonical_or_same(p)));
        let program_roots = program_roots();
        // Папка проекта, внутри которой лежит сама программа: команды там спрашиваются, как вне своей папки
        // (читающие идут без вопроса). Файлы проекта правит `write_file` без карточки, а не командная строка.
        let strict: Vec<PathBuf> = homes.iter().skip(1).filter(|h| program_roots.iter().any(|r| r.starts_with(h))).cloned().collect();
        Ctx { homes, workspace: self.workspace, data_dir: self.data_dir, program_roots, strict }
    }
}

/// Папки запущенной программы: её exe и (в отладочной сборке) папка `CREW_WEB_DIR`, которую страница читает с диска.
/// В них не пишет никто, даже внутри проекта, где бот иначе хозяин.
pub fn program_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(install) = crate::selfdev::install_dir() {
        out.push(canonical_or_same(&install));
    }
    if let Some(web) = crate::update::test_env("CREW_WEB_DIR").filter(|d| !d.is_empty()) {
        out.push(canonical_or_same(Path::new(&web)));
    }
    out
}

/// Файлы проекта, которые сами исполняются на этом компьютере без действия бота: хуки git, конфигурация git
/// (`core.hooksPath`, фильтры), атрибуты фильтров, настройки и хуки Claude Code, задачи редактора, профили оболочек,
/// конфиг cargo (`rustc-wrapper`). Запись туда — только после карточки.
pub fn auto_exec_reason(path: &Path) -> Option<&'static str> {
    let names: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let last = names.last().map(String::as_str).unwrap_or_default();
    if names.iter().any(|n| n == ".git") {
        return Some("файл, который сам запускает программы при работе с git");
    }
    if names.iter().any(|n| n == ".husky" || n == ".githooks") || last == ".gitattributes" {
        return Some("файл, который сам запускает программы при работе с git");
    }
    if let Some(i) = names.iter().position(|n| n == ".claude") {
        let rest = &names[i + 1..];
        if rest.iter().any(|n| n == "hooks") || (last.starts_with("settings") && last.ends_with(".json")) {
            return Some("настройки и хуки Claude Code: они запускают программы сами");
        }
    }
    if let Some(i) = names.iter().position(|n| n == ".vscode") {
        if matches!(names.get(i + 1).map(String::as_str), Some("tasks.json" | "launch.json" | "settings.json")) {
            return Some("настройки редактора, которые запускают программы");
        }
    }
    if let Some(i) = names.iter().position(|n| n == ".cargo") {
        if names.get(i + 1).is_some_and(|n| n.starts_with("config")) {
            return Some("конфигурация cargo, которая может подменить компилятор");
        }
    }
    if matches!(
        last,
        "profile.ps1" | "microsoft.powershell_profile.ps1" | "microsoft.vscode_profile.ps1" | ".bashrc" | ".bash_profile" | ".bash_login" | ".profile" | ".zshrc" | ".zprofile" | ".envrc"
    ) || names.iter().any(|n| n == "startup" || n == "start menu")
    {
        return Some("профиль оболочки или автозапуск");
    }
    // Настройки агентов и сборки, которые запускают программы сами или при первой же сборке.
    let after = |dir: &str| names.iter().position(|n| n == dir).map(|i| &names[i + 1..]);
    if last == ".mcp.json" || names.iter().any(|n| matches!(n.as_str(), ".codex" | ".gemini" | ".cursor")) {
        return Some("настройки агентов: они запускают программы");
    }
    if after(".claude").is_some_and(|rest| rest.iter().any(|n| n == "commands" || n == "agents")) {
        return Some("команды и агенты Claude Code");
    }
    if after(".github").is_some_and(|rest| rest.first().is_some_and(|n| n == "workflows")) {
        return Some("сценарии сборки на сервере");
    }
    if matches!(last, ".npmrc" | ".yarnrc" | ".yarnrc.yml" | ".gitmodules" | ".pre-commit-config.yaml" | "package.json" | "build.rs" | "makefile") {
        return Some("файл сборки или установки: он запускает программы");
    }
    // Другие агенты и редакторы с их хуками и командами.
    if after(".claude").is_some_and(|rest| rest.iter().any(|n| n == "skills"))
        || names.iter().any(|n| matches!(n.as_str(), ".opencode" | ".windsurf" | ".continue"))
        || last == "opencode.json"
        || last.starts_with(".aider")
    {
        return Some("настройки агентов и редакторов: они запускают программы");
    }
    // Голые репозитории (`x.git`) и папки, похожие на них: хуки и конфиг лежат прямо в корне.
    for (i, name) in names.iter().enumerate() {
        if name.ends_with(".git") && name != ".git" {
            let rest = &names[i + 1..];
            if rest.iter().any(|n| n == "hooks" || n == "attributes") || rest.last().is_some_and(|n| n == "config") {
                return Some("хуки и настройки репозитория git");
            }
        }
    }
    for ancestor in path.ancestors().skip(1) {
        if ancestor.join("HEAD").is_file() && ancestor.join("objects").is_dir() && ancestor.join("hooks").is_dir() {
            let rest: Vec<String> = path.strip_prefix(ancestor).map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()).unwrap_or_default();
            if rest.first().is_some_and(|n| n == "hooks" || n == "info") || rest.last().is_some_and(|n| n == "config") {
                return Some("хуки и настройки репозитория git");
            }
        }
    }
    // Не охраняются как обычные файлы проекта: `Cargo.toml` с `build =`, `conftest.py`, `setup.py`, `*.csproj`, `vite.config.*`:
    // без них проект не собрать, а запуск сборки и так спрашивается (в проекте с программой внутри команды оболочки всегда спрашивают).
    None
}

/// Расширения и имена файлов, которые запускаются сами или по двойному щелчку: создавать и перезаписывать их скриптом
/// без карточки нельзя в любой своей папке (подложенный `git.exe`, `x.lnk`, `.js` под Windows Script Host).
pub fn planted_executable(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem.to_string(), format!(".{ext}")),
        None => (name.clone(), String::new()),
    };
    if matches!(stem.as_str(), "pwsh" | "powershell" | "cmd") && ext.is_empty() {
        return true;
    }
    [
        ".exe", ".bat", ".cmd", ".com", ".ps1", ".psm1", ".psd1", ".dll", ".lnk", ".vbs", ".vbe", ".js", ".jse", ".wsf", ".wsh", ".msc", ".scr", ".pif",
        ".cpl", ".hta", ".jar", ".msi", ".reg", ".inf", ".url", ".scf",
    ]
    .contains(&ext.as_str())
}

/// Existing files with multiple hard links cannot be identified safely by pathname alone.
/// An unavailable link count is treated as unsafe. Missing files are safe only when the
/// metadata error is specifically NotFound.
pub fn single_link_or_missing(path: &Path) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => return e.kind() == std::io::ErrorKind::NotFound,
    };
    if meta.is_dir() { return true; }
    if !meta.is_file() { return false; }
    #[cfg(unix)]
    { use std::os::unix::fs::MetadataExt; return meta.nlink() == 1; }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let Ok(file) = std::fs::File::open(path) else { return false };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        return unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.is_ok() && info.nNumberOfLinks == 1;
    }
    #[cfg(not(any(unix, windows)))]
    { false }
}

/// Directory listings are unaffected; reading a file with uncertain identity is denied.
pub fn safe_read_links(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => true,
        Ok(_) => single_link_or_missing(path),
        Err(_) => false,
    }
}

/// Закрыто ли место в папке проекта или команды (не в своей папке бота): файлы автозапуска, документы для агентов
/// (`AGENTS.md`, `CLAUDE.md`) и сценарии в `tools/` и `scripts/`.
pub fn project_guard_reason(path: &Path) -> Option<&'static str> {
    if let Some(reason) = auto_exec_reason(path) {
        return Some(reason);
    }
    let names: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let last = names.last().map(String::as_str).unwrap_or_default();
    if matches!(last, "agents.md" | "claude.md") {
        return Some("инструкции для агентов: они читаются как указания");
    }
    let script = [".ps1", ".cmd", ".bat", ".sh"].iter().any(|ext| last.ends_with(ext));
    if script && names.iter().rev().skip(1).any(|n| n == "tools" || n == "scripts") {
        return Some("сценарии проекта: их запускает человек или сборка");
    }
    None
}

/// То же с папками проектов.
pub fn command_needs_approval_in(command: &str, places: &Places) -> bool {
    analyze(command, &places.ctx(), false, 0)
}

/// Чем опасна запись по этому пути, если он в папке запущенной программы (и не в своей папке).
pub fn install_reason(real: &Path, places: &Places) -> Option<&'static str> {
    let install = canonical_or_same(places.install?);
    let ctx = places.ctx();
    // Исключение — только своя папка бота: папка проекта команды может накрывать программу, и её файлы не отдаём.
    if real.starts_with(&install) && !real.starts_with(&ctx.homes[0]) {
        return Some(INSTALL_REFUSAL);
    }
    None
}

/// Отказ на изменение запущенной программы.
pub const INSTALL_REFUSAL: &str =
    "это папка запущенной программы Deloist, менять её нельзя: собери в checkout и обнови через выпуск";
/// Отказ на сборку мимо `target` самого checkout.
pub const TARGET_REFUSAL: &str =
    "сборка только в target самого checkout: CARGO_TARGET_DIR и --target-dir не задавай (общие папки *-target в C:\\Users\\Public съедают диск)";

/// Команду нельзя выполнять вовсе, ни с разрешением, ни без. `self_dev` — бот в режиме разработки:
/// ему нельзя ещё и уводить сборку из checkout.
pub fn command_refusal(command: &str, places: &Places, self_dev: bool) -> Option<&'static str> {
    let lower = command.to_lowercase();
    if self_dev && (lower.contains("cargo_target_dir") || lower.contains("--target-dir")) {
        return Some(TARGET_REFUSAL);
    }
    let install = canonical_or_same(places.install?);
    let ctx = places.ctx();
    touches_install(command, &ctx, &install, false, 0).then_some(INSTALL_REFUSAL)
}

static GIT_PUSH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bgit(\.exe)?\b[^&|;\n]*\bpush\b").unwrap());
static GH_RELEASE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bgh(\.exe)?\b[^&|;\n]*\brelease\b").unwrap());
static RELEASE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\brelease\.ps1\b").unwrap());
static GITVERSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)gitverse").unwrap());

/// Выпуск наружу: `git push`, `gh release`, `tools/release.ps1`, зеркало GitVerse. Такое спрашивается
/// всегда, в любом режиме, и только «один раз»: запомнить ответ нельзя. Проверка по тексту,
/// поэтому лишняя карточка возможна (`git commit -m "push"`), пропуск — нет.
pub fn release_command(command: &str) -> Option<&'static str> {
    if RELEASE_SCRIPT.is_match(command) {
        Some("выпуск (tools/release.ps1)")
    } else if GITVERSE.is_match(command) {
        Some("зеркало GitVerse")
    } else if GH_RELEASE.is_match(command) {
        Some("выпуск на GitHub (gh release)")
    } else if GIT_PUSH.is_match(command) {
        Some("отправка в репозиторий (git push)")
    } else {
        None
    }
}

/// Меняет ли команда что-то в папке запущенной программы: запись через `>`, не читающая
/// команда с таким путём или стоящая там.
fn touches_install(command: &str, ctx: &Ctx, install: &Path, powershell: bool, enc_depth: u32) -> bool {
    let command = HARMLESS_REDIRECT.replace_all(command, " ");
    let mut cwd: Option<PathBuf> = Some(ctx.homes[0].clone());
    let inside = |p: &Path| p.starts_with(install) && !p.starts_with(&ctx.homes[0]);
    for segment in split_segments(&command, powershell) {
        let words = split_words(&segment, powershell);
        let Some(start) = words.iter().position(|w| !w.redirect) else { continue };
        let Some((offset, verb)) = command_name(&words[start..]) else { continue };
        let args = &words[start + offset + 1..];
        if verb == "powershell" || verb == "pwsh" {
            let encoded = has_encoded_flag(args);
            if let Some(inner) = powershell_inner(args, enc_depth) {
                let next = if encoded { enc_depth + 1 } else { enc_depth };
                if touches_install(&inner, ctx, install, true, next) {
                    return true;
                }
            }
            continue;
        }
        if !powershell && verb == "cmd" {
            if cmd_inner(args).is_some_and(|inner| touches_install(&inner, ctx, install, false, enc_depth)) {
                return true;
            }
            continue;
        }
        let read_only = if powershell {
            PS_READ_ONLY.contains(&verb.as_str())
        } else if verb == "git" {
            git_read_only(args)
        } else {
            READ_ONLY.contains(&verb.as_str())
        };
        if !read_only && cwd.as_deref().is_some_and(inside) {
            return true;
        }
        for word in args {
            if !(word.redirect || !read_only) {
                continue;
            }
            let text = word.text.trim_matches(|c| c == '(' || c == ')' || c == ',' || c == ';');
            let text = match text.split_once('=') {
                Some((_, value)) if text.starts_with('-') || !is_absolute_like(text) => value,
                _ => text,
            };
            if text.is_empty() || text.starts_with('-') || URL.is_match(&text.to_lowercase()) {
                continue;
            }
            if text.starts_with('/') && !text[1..].contains(['/', '\\']) {
                continue;
            }
            if resolve(text, cwd.as_deref(), &ctx.homes[0]).as_deref().is_some_and(inside) {
                return true;
            }
        }
        let navigates = matches!(verb.as_str(), "cd" | "chdir" | "pushd" | "set-location" | "sl");
        if navigates {
            if let Some(target) = args.iter().find(|w| !w.redirect && !w.text.starts_with('/')) {
                cwd = resolve(&target.text, cwd.as_deref(), &ctx.homes[0]);
            }
        }
    }
    false
}

/// Секреты в команде: слово, ведущее к ним, и причина. Проверка для всех режимов, в том числе
/// «свобода действий» и strict: секреты закрыты всегда, карточкой их не открыть. Всё, что
/// понять нельзя (подстановки, чужие диски), здесь не отказ: этим занимается режим «спрашивать».
pub fn command_secret(command: &str, workspace: &Path, data_dir: &Path) -> Option<(String, &'static str)> {
    let ctx = Ctx { homes: vec![canonical_or_same(workspace)], workspace, data_dir, program_roots: Vec::new(), strict: Vec::new() };
    find_secret(command, &ctx, false, 0)
}

fn find_secret(command: &str, ctx: &Ctx, powershell: bool, enc_depth: u32) -> Option<(String, &'static str)> {
    let command = HARMLESS_REDIRECT.replace_all(command, " ");
    let mut cwd: Option<PathBuf> = Some(ctx.homes[0].clone());
    for segment in split_segments(&command, powershell) {
        let words = split_words(&segment, powershell);
        let Some(start) = words.iter().position(|w| !w.redirect) else { continue };
        let Some((offset, verb)) = command_name(&words[start..]) else { continue };
        let args = &words[start + offset + 1..];
        if verb.len() == 2 && verb.ends_with(':') && verb.as_bytes()[0].is_ascii_alphabetic() {
            cwd = None;
            continue;
        }
        if verb == "powershell" || verb == "pwsh" {
            let encoded = has_encoded_flag(args);
            match powershell_inner(args, enc_depth) {
                Some(inner) => {
                    let next = if encoded { enc_depth + 1 } else { enc_depth };
                    if let Some(found) = find_secret(&inner, ctx, true, next) {
                        return Some(found);
                    }
                }
                None if encoded => {
                    return Some((encoded_value(args).unwrap_or_else(|| command.to_string()), ENCODED_UNCHECKABLE));
                }
                None => {}
            }
        }
        if !powershell && verb == "cmd" {
            if let Some(found) = cmd_inner(args).and_then(|inner| find_secret(&inner, ctx, false, enc_depth)) {
                return Some(found);
            }
        }
        for word in args {
            if let Place::Secret(reason) = classify(word, cwd.as_deref(), ctx) {
                return Some((word.text.clone(), reason));
            }
        }
        let navigates = matches!(verb.as_str(), "cd" | "chdir" | "pushd" | "set-location" | "sl");
        if navigates {
            if let Some(target) = args.iter().find(|w| !w.redirect && !w.text.starts_with('/')) {
                cwd = resolve(&target.text, cwd.as_deref(), &ctx.homes[0]);
            }
        } else if verb == "popd" {
            cwd = None;
        }
    }
    None
}

/// Удаляют файлы.
const DELETES: &[&str] = &["rm", "del", "erase", "rmdir", "rd", "remove-item", "ri", "rimraf", "shred"];
/// Отправляют данные наружу.
const SENDS: &[&str] = &[
    "curl", "wget", "scp", "sftp", "ssh", "ftp", "rsync", "invoke-webrequest", "iwr", "invoke-restmethod", "irm",
    "send-mailmessage",
];
/// Запускают что-то, чьё имя не видно (оболочки, интерпретаторы, запускатели, перебор файлов):
/// такую строку разрешают только один раз.
const OPAQUE: &[&str] = &[
    "cmd", "powershell", "pwsh", "start", "call", "bash", "sh", "wsl", "runas", "env", "node", "python", "python3",
    "py", "pythonw", "npx", "pnpx", "bunx", "xargs", "find", "forfiles", "certutil", "mshta", "rundll32", "regsvr32",
    "cscript", "wscript", "deno", "bun", "uv", "uvx", "robocopy", "move", "mv", "perl", "ruby", "php", "java",
    "dotnet", "msbuild", "make", "ssh-agent", "schtasks", "at", "explorer", "tsx", "ts-node", "electron", "osascript",
    "rscript", "lua", "julia", "awk", "gawk", "jshell", "groovy", "pythonw", "ipython",
];
/// Ключи git, после которых идёт значение: их пропускаем вместе со значением.
const GIT_VALUE_FLAGS: &[&str] = &["-c", "--git-dir", "--work-tree", "--namespace", "--exec-path", "--super-prefix", "--config-env"];

/// Подкоманда git: первое слово, которое не ключ и не значение ключа. `None` — git с
/// подменой команд (`-c alias.*`, `-c core.sshCommand`, `--exec-path`) или без подкоманды.
fn git_subcommand(args: &[String]) -> Option<(String, Vec<String>)> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some((flag, _)) = a.split_once('=') {
            if GIT_VALUE_FLAGS.contains(&flag) {
                if flag == "--exec-path" || flag == "--config-env" {
                    return None;
                }
                i += 1;
                continue;
            }
        }
        if GIT_VALUE_FLAGS.contains(&a) {
            let value = args.get(i + 1).map(String::as_str).unwrap_or("");
            // Псевдоним или своя программа вместо git: что запустится, по строке не узнать.
            if a == "--exec-path" || a == "--config-env" || (a == "-c" && (value.starts_with("alias.") || value.contains("command") || value.contains("editor") || value.contains("pager"))) {
                return None;
            }
            i += 2;
            continue;
        }
        if !a.starts_with('-') {
            return Some((a.to_string(), args[i + 1..].to_vec()));
        }
        i += 1;
    }
    None
}

/// Место команды для разрешения на разговор или навсегда — вся строка целиком (пробелы
/// схлопнуты) и вид (`delete`, `publish`, `send`, `secure` или пусто). Правило на
/// `del C:\proj\tmp.log` не открывает `del /s /q C:\Users\…`: другая строка — другое место.
/// `None` — строку нельзя узнать: подстановки, вложенная оболочка или интерпретатор, нет имени.
/// Её разрешают только один раз.
pub fn command_place(command: &str) -> Option<(String, &'static str)> {
    if command.chars().any(|c| matches!(c, '%' | '$' | '`' | '(' | ')' | '{' | '}' | '^' | '!' | '*' | '?')) {
        return None;
    }
    let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() || normalized.chars().count() > 500 {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    let mut kind = "";
    let rank = |k: &str| match k {
        "secure" => 4,
        "delete" => 3,
        "publish" => 2,
        "send" => 1,
        _ => 0,
    };
    for segment in split_segments(command, false) {
        let words = split_words(&segment, false);
        let start = words.iter().position(|w| !w.redirect)?;
        let (offset, name) = command_name(&words[start..])?;
        if name.is_empty() || OPAQUE.contains(&name.as_str()) || name.ends_with(':') {
            return None;
        }
        // Сценарий из файла — сам по себе или аргументом любой программы: файл могут переписать.
        if words[start + offset..].iter().any(|w| is_script_file(&w.text)) && name != "git" {
            return None;
        }
        if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return None;
        }
        let args: Vec<String> = words[start + offset + 1..].iter().map(|w| w.text.to_lowercase()).collect();
        // Подкоманда: первое слово не ключ.
        let (first, rest) = if name == "git" {
            git_subcommand(&args)?
        } else {
            let at = args.iter().position(|a| !a.starts_with('-'));
            match at {
                Some(at) => (args[at].clone(), args[at + 1..].to_vec()),
                None => (String::new(), Vec::new()),
            }
        };
        let first = first.as_str();
        // Запуск произвольного кода через менеджер пакетов — как интерпретатор.
        let runs_code = runs_foreign_code(&name, first) || (name == "git" && matches!(first, "submodule" | "filter-branch" | "bisect"));
        if runs_code {
            return None;
        }
        let has = |flag: &str| rest.iter().any(|a| a == flag);
        let this = if SYSTEM_WIDE.contains(&name.as_str()) {
            "secure"
        } else if DELETES.contains(&name.as_str())
            || (name == "git" && matches!(first, "rm" | "clean"))
            || (name == "git" && first == "reset" && has("--hard"))
            || (name == "git" && first == "branch" && (has("-d") || has("--delete") || has("-D")))
            || (name == "git" && first == "push" && (has("--force") || has("-f") || has("--delete")))
        {
            "delete"
        } else if (name == "git" && first == "push")
            || (matches!(name.as_str(), "npm" | "yarn" | "pnpm" | "cargo" | "twine" | "gh") && matches!(first, "publish" | "release" | "upload"))
        {
            "publish"
        } else if SENDS.contains(&name.as_str()) {
            "send"
        } else {
            ""
        };
        if rank(this) > rank(kind) {
            kind = this;
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return None;
    }
    Some((normalized, kind))
}

struct Ctx<'a> {
    /// Своя папка первой, за ней папки проектов; все канонические.
    homes: Vec<PathBuf>,
    workspace: &'a Path,
    data_dir: &'a Path,
    /// Папки запущенной программы: закрыты и внутри папок проектов.
    program_roots: Vec<PathBuf>,
    /// Папки проектов с программой внутри: команды в них спрашиваются.
    strict: Vec<PathBuf>,
}

/// Куда ведёт слово.
enum Place {
    /// Своя папка или вовсе не путь.
    Home,
    Outside,
    /// Секрет и чем он опасен (текст из [`secret_reason`]).
    Secret(&'static str),
    /// Понять нельзя: сетевой путь, `~`, диск PowerShell, папка после `d:`.
    Unknown,
}

fn analyze(command: &str, ctx: &Ctx, powershell: bool, enc_depth: u32) -> bool {
    // Подстановки окружения меняют путь после нашей проверки: не гадаем.
    let lower = command.to_lowercase();
    if lower.contains('%') || lower.contains("$env:") || lower.contains("$home") || lower.contains('`') {
        return true;
    }
    if powershell && (lower.contains('$') || lower.contains('{') || lower.contains('(')) {
        return true;
    }
    let command = HARMLESS_REDIRECT.replace_all(command, " ");

    // Где стоит команда: `None` — ушли туда, куда не видно.
    let mut cwd: Option<PathBuf> = Some(ctx.homes[0].clone());
    for segment in split_segments(&command, powershell) {
        let words = split_words(&segment, powershell);
        let Some(start) = words.iter().position(|w| !w.redirect) else { continue };
        let Some((offset, verb)) = command_name(&words[start..]) else { continue };
        let at = start + offset;
        let args = &words[at + 1..];

        if verb.len() == 2 && verb.ends_with(':') && verb.as_bytes()[0].is_ascii_alphabetic() {
            // `d:` — смена диска.
            cwd = None;
            continue;
        }
        if SYSTEM_WIDE.contains(&verb.as_str()) {
            return true;
        }
        if verb == "powershell" || verb == "pwsh" {
            let encoded = has_encoded_flag(args);
            match powershell_inner(args, enc_depth) {
                Some(inner) => {
                    let next = if encoded { enc_depth + 1 } else { enc_depth };
                    if !analyze(&inner, ctx, true, next) {
                        continue;
                    } else {
                        return true;
                    }
                }
                None => return true,
            }
        }
        if !powershell && verb == "cmd" {
            match cmd_inner(args) {
                Some(inner) if !analyze(&inner, ctx, false, enc_depth) => continue,
                _ => return true,
            }
        }

        let read_only = if powershell {
            PS_READ_ONLY.contains(&verb.as_str())
        } else if verb == "git" {
            git_read_only(args)
        } else {
            READ_ONLY.contains(&verb.as_str())
        };
        if powershell && !read_only {
            return true;
        }
        let at_home = cwd.as_deref().is_some_and(|dir| at_home(dir, ctx));
        if !read_only && !at_home {
            // Стоим не в своей папке, а команда что-то меняет.
            return true;
        }

        for word in args {
            match classify(word, cwd.as_deref(), ctx) {
                Place::Home => {}
                Place::Secret(_) | Place::Unknown => return true,
                // Запись через `>` — это изменение, даже у читающей команды.
                Place::Outside if word.redirect || !read_only => return true,
                Place::Outside => {}
            }
        }

        let navigates = matches!(verb.as_str(), "cd" | "chdir" | "pushd" | "set-location" | "sl");
        if navigates {
            if let Some(target) = args.iter().find(|w| !w.redirect && !w.text.starts_with('/')) {
                cwd = resolve(&target.text, cwd.as_deref(), &ctx.homes[0]);
            }
        } else if verb == "popd" {
            cwd = None;
        }
    }
    false
}

/// Скрипт внутри `powershell`/`pwsh` в строке команды (с раскодированным `-EncodedCommand`) — для карточки и описания.
pub(crate) fn inner_shell_script(command: &str) -> Option<String> {
    for segment in split_segments(command, false) {
        let words = split_words(&segment, false);
        let start = words.iter().position(|w| !w.redirect)?;
        let (offset, verb) = command_name(&words[start..])?;
        if verb == "powershell" || verb == "pwsh" {
            return powershell_inner(&words[start + offset + 1..], 0);
        }
        return None;
    }
    None
}

fn git_read_only(args: &[Word]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let w = args[i].text.to_lowercase();
        if w == "-c" {
            // `-C <папка>` и `-c ключ=значение`.
            i += 2;
            continue;
        }
        if w.starts_with('-') {
            i += 1;
            continue;
        }
        return GIT_READ_ONLY.contains(&w.as_str());
    }
    false
}

/// Расширения, с которыми Windows запускает файл по имени: `npx.cmd` — это `npx`.
const RUNNABLE_EXTS: &[&str] = &[".exe", ".com", ".cmd", ".bat", ".ps1", ".vbs"];
/// Файлы-сценарии: бот может переписать такой файл после разрешения, поэтому их запуск
/// (сам по себе или через любой интерпретатор) разрешают только один раз.
const SCRIPT_EXTS: &[&str] = &[
    ".bat", ".cmd", ".ps1", ".psm1", ".vbs", ".vbe", ".js", ".jse", ".mjs", ".cjs", ".ts", ".wsf", ".wsh", ".py", ".pyw", ".sh",
    ".rb", ".pl", ".php", ".lua", ".r",
];
/// Известные программы, которые на Windows ставятся обёрткой `.cmd`/`.ps1` (`npm.cmd`): это не сценарий бота.
const KNOWN_SHIMS: &[&str] = &["npm", "pnpm", "yarn", "npx", "pnpx", "node", "code", "gh", "git", "cargo", "pip", "pip3", "tsc", "ng"];

/// Имя команды без пути и без `.exe`, `.com`, `.cmd`, `.bat`, `.ps1`, `.vbs`; `@` и `call` пропускаем.
fn command_name(words: &[Word]) -> Option<(usize, String)> {
    for (i, word) in words.iter().enumerate() {
        let raw = word.text.trim_start_matches('@').to_lowercase();
        if raw == "call" || raw.is_empty() {
            continue;
        }
        let name = Path::new(&raw).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(raw.clone());
        let name = RUNNABLE_EXTS.iter().find_map(|ext| name.strip_suffix(ext)).map(str::to_string).unwrap_or(name);
        return Some((i, name));
    }
    None
}

/// Слово — файл-сценарий (`run.bat`, `.\x.ps1`, `x.js`), а не обёртка известной программы.
fn is_script_file(text: &str) -> bool {
    let lower = text.trim_matches(|c| c == '"' || c == '\'').to_lowercase();
    let Some(ext) = SCRIPT_EXTS.iter().find(|ext| lower.ends_with(*ext)) else { return false };
    let file = Path::new(&lower).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = file.strip_suffix(ext).unwrap_or_default();
    !stem.is_empty() && !KNOWN_SHIMS.contains(&stem)
}

/// Сборка и установка пакетов запускают чужой код (сценарии пакетов, `build.rs`, цели make):
/// что выполнится, по строке не узнать.
fn runs_foreign_code(name: &str, first: &str) -> bool {
    match name {
        "npm" | "pnpm" | "yarn" => {
            // Голый `yarn` и `pnpm` — это установка.
            (first.is_empty() && name != "npm")
                || matches!(
                    first,
                    "install" | "i" | "ci" | "add" | "update" | "up" | "upgrade" | "rebuild" | "run" | "run-script" | "test" | "t"
                        | "start" | "restart" | "stop" | "exec" | "x" | "dlx" | "create" | "init" | "link" | "pack"
                )
        }
        "cargo" => matches!(first, "build" | "b" | "test" | "t" | "run" | "r" | "install" | "script" | "bench" | "check" | "c" | "clippy" | "doc" | "fix"),
        "pip" | "pip3" => matches!(first, "install" | "download" | "wheel"),
        "make" | "nmake" | "gmake" | "gradle" | "gradlew" | "mvn" | "mvnw" | "ant" | "ninja" | "cmake" | "msbuild" | "meson" | "bazel" => true,
        "dotnet" => true,
        _ => false,
    }
}

/// Тело `cmd /c "…"`. Без `/c` или `/k` cmd читает команды из входа: не разбираем.
fn cmd_inner(args: &[Word]) -> Option<String> {
    let at = args.iter().position(|w| matches!(w.text.to_lowercase().as_str(), "/c" | "/k" | "/r"))?;
    let rest = &args[at + 1..];
    if rest.len() == 1 && rest[0].quoted {
        return Some(rest[0].text.clone());
    }
    Some(rest.iter().map(Word::source).collect::<Vec<_>>().join(" "))
}

/// Тело `powershell -c "…"` или `powershell -enc <base64>`. Скрипты из файла и вход не разбираем.
fn powershell_inner(args: &[Word], enc_depth: u32) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let (name, inline) = split_flag(&args[i].text);
        if !name.starts_with('-') {
            // Без ключа PowerShell считает остаток командой.
            break;
        }
        if name == "-c" || name == "-command" {
            i += 1;
            break;
        }
        if name == "-" || name.starts_with("-f") || name.starts_with("-wd") || name.starts_with("-workingdirectory") {
            return None;
        }
        if is_encoded_flag(&name) {
            if enc_depth >= 3 {
                return None;
            }
            let value = if let Some(v) = inline {
                v
            } else {
                i += 1;
                args.get(i)?.text.clone()
            };
            return decode_encoded_command(&value);
        }
        i += if PS_VALUE_FLAGS.contains(&name.as_str()) { 2 } else { 1 };
    }
    let rest = args.get(i..)?;
    if rest.is_empty() {
        return None;
    }
    if rest.len() == 1 && rest[0].quoted {
        return Some(rest[0].text.clone());
    }
    Some(rest.iter().map(Word::source).collect::<Vec<_>>().join(" "))
}

/// Декодирует `-EncodedCommand`: base64 → UTF-16LE → текст скрипта.
fn decode_encoded_command(encoded: &str) -> Option<String> {
    let bytes = STANDARD.decode(encoded.trim()).ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16(&units).ok()
}

/// Ключ PowerShell в нижнем регистре и значение после `:` как есть (base64 чувствителен к регистру).
fn split_flag(text: &str) -> (String, Option<String>) {
    match text.split_once(':') {
        Some((n, v)) => (n.to_lowercase(), Some(v.to_string())),
        None => (text.to_lowercase(), None),
    }
}

/// `-e`, `-en` … `-encodedcommand` и отдельное сокращение `-ec`.
fn is_encoded_flag(name: &str) -> bool {
    name == "-ec" || (name.starts_with("-e") && "-encodedcommand".starts_with(name))
}

fn has_encoded_flag(args: &[Word]) -> bool {
    args.iter().any(|w| is_encoded_flag(&split_flag(&w.text).0))
}

fn encoded_value(args: &[Word]) -> Option<String> {
    for (i, w) in args.iter().enumerate() {
        let (name, inline) = split_flag(&w.text);
        if is_encoded_flag(&name) {
            if let Some(v) = inline {
                return Some(v);
            }
            return args.get(i + 1).map(|x| x.text.clone());
        }
    }
    None
}

/// Куда ведёт путь из команды. `\x` — от корня того же диска, где папка бота.
fn resolve(raw: &str, cwd: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let path = Path::new(raw);
    let has_prefix = path.components().any(|c| matches!(c, Component::Prefix(_)));
    let has_root = path.has_root();
    if has_prefix && has_root {
        return real_path(path);
    }
    if has_root {
        let mut on_drive: PathBuf = home.components().take_while(|c| matches!(c, Component::Prefix(_))).collect();
        on_drive.push(path);
        return real_path(&on_drive);
    }
    if has_prefix {
        // `C:file` — относительно текущей папки другого диска.
        return None;
    }
    real_path(&cwd?.join(path))
}

fn classify(word: &Word, cwd: Option<&Path>, ctx: &Ctx) -> Place {
    let text = word.text.trim_matches(|c| c == '(' || c == ')' || c == ',' || c == ';');
    // `--out=..\x`: путь — то, что после `=`.
    let text = match text.split_once('=') {
        Some((_, value)) if text.starts_with('-') => value,
        _ => text,
    };
    if text.is_empty() {
        return Place::Home;
    }
    let lower = text.to_lowercase();
    // Адрес в сети — не путь на диске.
    if URL.is_match(&lower) {
        return Place::Home;
    }
    // Текст в кавычках с пробелами — сообщение, а не путь (`git commit -m "fix: C:\ в тексте"`).
    let bytes = text.as_bytes();
    let starts_like_path = text.starts_with(['\\', '/', '~', '.'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    if word.quoted && text.contains(char::is_whitespace) && !starts_like_path {
        return Place::Home;
    }
    if let Some(found) = DRIVE.captures(text).and_then(|c| c.get(1)) {
        let path = found.as_str();
        if path.len() == 2 {
            return Place::Unknown;
        }
        return place_of(real_path(Path::new(path)), ctx);
    }
    if lower.contains(":\\") || lower.contains(":/") {
        // `hklm:\…`, `cert:\…` и прочие диски PowerShell.
        return Place::Unknown;
    }
    if text.starts_with("\\\\") || text.starts_with('~') {
        return Place::Unknown;
    }
    if let Some(body) = text.strip_prefix('/') {
        // Ключи cmd: `/s`, `/b`, `/c:"текст"`. Путь в стиле unix — `/c/Users/…`.
        if body.is_empty() || body.contains(['/', '\\']) {
            return Place::Unknown;
        }
        return Place::Home;
    }
    if text.starts_with('\\') {
        return place_of(resolve(text, cwd, &ctx.homes[0]), ctx);
    }
    if text.starts_with('-') && !text.contains("..") {
        return Place::Home;
    }
    let leaves = text.split(['\\', '/']).any(|part| part == "..");
    match cwd {
        // В своей папке секретов нет. В папке проекта они могут быть (`data` рядом с Cargo.toml):
        // там путь проверяется целиком.
        Some(dir) if dir.starts_with(&ctx.homes[0]) && !leaves => Place::Home,
        Some(dir) => place_of(real_path(&dir.join(text)), ctx),
        None => Place::Unknown,
    }
}

fn place_of(real: Option<PathBuf>, ctx: &Ctx) -> Place {
    let Some(real) = real else { return Place::Unknown };
    if let Some(reason) = secret_reason(&real, ctx.data_dir, ctx.workspace) {
        return Place::Secret(reason);
    }
    if at_home(&real, ctx) {
        Place::Home
    } else {
        Place::Outside
    }
}

fn at_home(path: &Path, ctx: &Ctx) -> bool {
    ctx.homes.iter().enumerate().any(|(i, home)| {
        path.starts_with(home)
            && (i == 0
                || (!ctx.strict.contains(home)
                    && !ctx.program_roots.iter().any(|r| path.starts_with(r))
                    && project_guard_reason(path).is_none()))
    })
}

// ---------------------------------------------------------------------------
// Разбор строки
// ---------------------------------------------------------------------------

struct Word {
    text: String,
    /// Слово стоит после `>`: туда пишут.
    redirect: bool,
    quoted: bool,
}

impl Word {
    /// Слово так, как его надо передать дальше.
    fn source(&self) -> String {
        if self.quoted {
            format!("\"{}\"", self.text)
        } else {
            self.text.clone()
        }
    }
}

/// Делим по `&`, `|`, `;` и переводу строки вне кавычек.
fn split_segments(command: &str, powershell: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None if c == '"' || (powershell && c == '\'') => quote = Some(c),
            None if matches!(c, '&' | '|' | ';' | '\n' | '\r') => {
                out.push(std::mem::take(&mut current));
                continue;
            }
            None => {}
        }
        current.push(c);
    }
    out.push(current);
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

/// Слова с учётом кавычек; `>` и `<` — разделители, слово после `>` помечено.
fn split_words(segment: &str, powershell: bool) -> Vec<Word> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut quote: Option<char> = None;
    let mut redirect = false;
    let flush = |current: &mut String, quoted: &mut bool, redirect: &mut bool, out: &mut Vec<Word>| {
        if !current.is_empty() || *quoted {
            out.push(Word { text: std::mem::take(current), redirect: *redirect, quoted: *quoted });
            *redirect = false;
            *quoted = false;
        }
    };
    for c in segment.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || (powershell && c == '\'') => {
                quote = Some(c);
                quoted = true;
            }
            None if c == '>' => {
                // Цифра перед `>` — номер потока, а не слово.
                if !current.is_empty() && current.chars().all(|d| d.is_ascii_digit()) && !quoted {
                    current.clear();
                } else {
                    flush(&mut current, &mut quoted, &mut redirect, &mut out);
                }
                redirect = true;
            }
            None if c == '<' || c.is_whitespace() => flush(&mut current, &mut quoted, &mut redirect, &mut out),
            None => current.push(c),
        }
    }
    flush(&mut current, &mut quoted, &mut redirect, &mut out);
    out
}

// reach parses cmd.exe/PowerShell; a bash equivalent is stage-3 work.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        data: PathBuf,
        work: PathBuf,
        other: PathBuf,
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("deloist-reach-{name}-{}", std::process::id()));
        let data = root.join("data");
        let work = data.join("workspaces").join("bot1");
        let other = root.join("project");
        std::fs::create_dir_all(work.join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(data.join("secrets.json"), "{}").unwrap();
        std::fs::write(data.join("runtime.json"), "{}").unwrap();
        std::fs::write(other.join("readme.md"), "hi").unwrap();
        Fixture { root, data, work, other }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn asks(f: &Fixture, command: &str) -> bool {
        command_needs_approval(command, &f.work, &f.data)
    }

    #[test]
    fn anything_inside_the_own_folder_runs_freely() {
        let f = fixture("home");
        let own = format!("type \"{}\" && del \"{}\"", f.work.join("a.txt").display(), f.work.join("b.txt").display());
        for command in [
            "echo один",
            "ver",
            "npm install",
            "node build.js > out.txt 2>&1",
            "del /q *.tmp",
            "mkdir sub\\x && cd sub && echo ok > a.txt",
            "git init && git add . && git commit -m \"fix: C:\\ в тексте\"",
            "python -c \"print(1)\"",
            "curl -L https://example.com/file.zip -o file.zip",
            "set",
            "dir > nul 2>&1",
            &own,
        ] {
            assert!(!asks(&f, command), "{command}");
        }
    }

    #[test]
    fn reading_elsewhere_is_free() {
        let f = fixture("read");
        let other = f.other.display().to_string();
        for command in [
            format!("dir \"{other}\""),
            format!("type \"{other}\\readme.md\""),
            format!("dir /s /b \"{other}\" | findstr readme"),
            format!("tree \"{other}\" /f"),
            format!("git -C \"{other}\" status"),
            format!("git -C \"{other}\" log --oneline -5"),
            format!("powershell -NoProfile -c \"Get-ChildItem '{other}' -Recurse\""),
            format!("powershell -ex bypass -c \"Get-Content '{other}\\readme.md'\""),
            format!("cd /d \"{other}\" && dir"),
            format!("cmd /c \"type {other}\\readme.md\""),
            "dir C:\\".to_string(),
        ] {
            assert!(!asks(&f, &command), "{command}");
        }
    }

    #[test]
    fn changing_things_elsewhere_asks() {
        let f = fixture("write");
        let other = f.other.display().to_string();
        for command in [
            format!("del \"{other}\\readme.md\""),
            format!("echo x > \"{other}\\new.txt\""),
            format!("echo x>\"{other}\\new.txt\""),
            format!("cd /d \"{other}\" && del readme.md"),
            format!("cd /d \"{other}\" && make"),
            format!("copy readme.md \"{other}\""),
            format!("git -C \"{other}\" commit -am x"),
            format!("powershell -c \"Remove-Item '{other}' -Recurse\""),
            format!("powershell -ex bypass -c \"Remove-Item '{other}'\""),
            format!("powershell -c \"Get-ChildItem '{other}' | Remove-Item\""),
            format!("cmd /c \"del {other}\\readme.md\""),
            format!("type x.bat | cmd"),
            "rmdir /s /q ..\\..".to_string(),
            "cd .. && del /q *".to_string(),
            "d: && del /q *".to_string(),
            "del %USERPROFILE%\\x".to_string(),
            "powershell -File evil.ps1".to_string(),
            "powershell -c \"$p='x'; rm $p\"".to_string(),
            "shutdown /s /t 0".to_string(),
            "taskkill /f /im explorer.exe".to_string(),
            "reg add HKCU\\Software\\x".to_string(),
            "del \\\\server\\share\\x".to_string(),
            "type \\\\localhost\\c$\\x".to_string(),
            "(cd C:\\ & del *)".to_string(),
        ] {
            assert!(asks(&f, &command), "{command}");
        }
    }

    #[test]
    fn secrets_ask_even_for_reading() {
        let f = fixture("secret");
        for command in [
            "type ..\\..\\secrets.json".to_string(),
            "type ..\\..\\runtime.json".to_string(),
            "node x.js --out=..\\..\\secrets.json".to_string(),
            format!("type \"{}\"", f.data.join("runtime.json").display()),
            format!("dir \"{}\"", f.data.display()),
            format!("cd /d \"{}\" && type runtime.json", f.data.display()),
            "type C:\\Users\\someone\\.ssh\\id_ed25519".to_string(),
        ] {
            assert!(asks(&f, &command), "{command}");
        }
        // Тот же путь от корня диска без буквы.
        let rooted: String = f.data.join("runtime.json").display().to_string().chars().skip(2).collect();
        assert!(asks(&f, &format!("type \"{rooted}\"")), "{rooted}");
    }

    #[test]
    fn secret_paths_are_recognised() {
        let f = fixture("reason");
        let real = |p: &Path| real_path(p).unwrap();
        assert!(matches!(secret_reason(&f.other.join("deloist").join("secrets.key"), &f.data, &f.work), Some(r) if r != KEY_FILE), "мастер-ключ закрыт без карточки");
        assert!(secret_reason(&real(&f.data.join("runtime.json")), &f.data, &f.work).is_some());
        assert!(secret_reason(&real(&f.work.join("notes.md")), &f.data, &f.work).is_none());
        assert!(secret_reason(&real(&f.other.join("readme.md")), &f.data, &f.work).is_none());
        assert!(secret_reason(&f.other.join(".env"), &f.data, &f.work).is_some());
        assert!(secret_reason(Path::new(r"C:\Users\u\AppData\Local\Google\Chrome\User Data\Default\Login Data"), &f.data, &f.work).is_some());
        assert!(secret_reason(Path::new(r"C:\Users\u\.ssh\id_rsa"), &f.data, &f.work).is_some());
    }

    fn checkout(f: &Fixture) -> PathBuf {
        let dir = f.root.join("checkout");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"deloist\"\n").unwrap();
        dir
    }

    #[test]
    fn a_project_folder_has_the_rights_of_the_own_folder() {
        let f = fixture("project");
        let repo = checkout(&f);
        let projects = [repo.clone()];
        let with = Places { workspace: &f.work, projects: &projects, data_dir: &f.data, install: None };
        let r = repo.display().to_string();
        for command in [
            format!("cd /d \"{r}\" && cargo test --lib reach"),
            format!("cd /d \"{r}\" && git checkout -b feat/x && git commit -am x"),
            format!("echo x > \"{r}\\src\\a.rs\""),
            format!("cd /d \"{r}\" && node tools/check_i18n.mjs"),
        ] {
            assert!(command_needs_approval(&command, &f.work, &f.data), "без режима спрашивает: {command}");
            assert!(!command_needs_approval_in(&command, &with), "в режиме своё: {command}");
        }
        // Чужое рядом с проектом — по-прежнему чужое.
        let other = f.other.display().to_string();
        assert!(command_needs_approval_in(&format!("del \"{other}\\readme.md\""), &with));
    }

    #[test]
    fn data_inside_a_project_stays_secret() {
        // Отладочная сборка из checkout держит данные в `<checkout>\data`.
        let f = fixture("project-data");
        let repo = checkout(&f);
        let data = repo.join("data");
        let work = data.join("workspaces").join("bot1");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(data.join("runtime.json"), "{}").unwrap();
        let projects = [repo.clone()];
        let with = Places { workspace: &work, projects: &projects, data_dir: &data, install: None };
        let r = repo.display().to_string();
        assert!(command_needs_approval_in(&format!("cd /d \"{r}\" && type data\\runtime.json"), &with));
        assert!(command_needs_approval_in(&format!("type \"{r}\\data\\runtime.json\""), &with));
        assert!(secret_reason(&real_path(&data.join("runtime.json")).unwrap(), &data, &work).is_some());
        assert!(!command_needs_approval_in(&format!("cd /d \"{r}\" && type Cargo.toml"), &with));
    }

    #[test]
    fn the_running_install_is_never_changed() {
        let f = fixture("install");
        let install = f.root.join("install");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("deloist.exe"), "x").unwrap();
        let places = Places { workspace: &f.work, projects: &[], data_dir: &f.data, install: Some(&install) };
        let i = install.display().to_string();
        for command in [
            format!("copy deloist.exe \"{i}\\deloist.exe\""),
            format!("echo x > \"{i}\\deloist.exe\""),
            format!("cd /d \"{i}\" && del deloist.exe"),
            format!("cmd /c \"del {i}\\deloist.exe\""),
            format!("powershell -c \"Remove-Item '{i}\\deloist.exe'\""),
        ] {
            assert_eq!(command_refusal(&command, &places, false), Some(INSTALL_REFUSAL), "{command}");
        }
        for command in [format!("type \"{i}\\deloist.exe\""), format!("dir \"{i}\""), "echo x > a.txt".to_string()] {
            assert_eq!(command_refusal(&command, &places, false), None, "{command}");
        }
        assert!(install_reason(&real_path(&install.join("deloist.exe")).unwrap(), &places).is_some());
        // Установка вокруг данных (`C:\Users\Public\Deloist\data`): своя папка внутри неё — своя.
        let root_install = Places { workspace: &f.work, projects: &[], data_dir: &f.data, install: Some(&f.root) };
        assert_eq!(command_refusal("echo x > a.txt", &root_install, false), None);
        assert!(install_reason(&real_path(&f.work.join("a.txt")).unwrap(), &root_install).is_none());
    }

    #[test]
    fn builds_stay_in_the_checkout_target() {
        let f = fixture("target");
        let places = Places { workspace: &f.work, projects: &[], data_dir: &f.data, install: None };
        for command in [
            "set CARGO_TARGET_DIR=C:\\Users\\Public\\x-target && cargo build",
            "cargo test --target-dir C:\\Users\\Public\\crew-target",
        ] {
            assert_eq!(command_refusal(command, &places, true), Some(TARGET_REFUSAL), "{command}");
            assert_eq!(command_refusal(command, &places, false), None, "вне режима не трогаем: {command}");
        }
        assert_eq!(command_refusal("cargo test", &places, true), None);
    }

    #[test]
    fn publishing_is_recognised() {
        for command in [
            "git push",
            "git -C C:\\repo push origin main",
            "cd x && git push --tags",
            "cmd /c \"git push\"",
            "gh release create v0.3.1",
            "pwsh -File tools/release.ps1",
            "powershell -ExecutionPolicy Bypass -File .\\tools\\release.ps1 -Tag v1",
            "bash tools/mirror_gitverse.sh",
            "git remote add gv https://gitverse.ru/x/y.git",
        ] {
            assert!(release_command(command).is_some(), "{command}");
        }
        for command in ["git status", "git commit -m fix", "gh pr view 1", "cargo test", "git log --oneline"] {
            assert!(release_command(command).is_none(), "{command}");
        }
    }

    #[test]
    fn display_hides_the_verbatim_prefix() {
        assert_eq!(display(Path::new(r"\\?\C:\X\tripo")), r"C:\X\tripo");
        assert_eq!(display(Path::new(r"\\?\UNC\srv\share")), r"\\srv\share");
    }

    fn encode_ps(script: &str) -> String {
        let mut bytes = Vec::with_capacity(script.len() * 2);
        for unit in script.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    }

    #[test]
    fn encoded_secret_read_is_refused() {
        let f = fixture("enc-secret");
        let secret = f.data.join("secrets.json");
        let script = format!("Get-Content '{}'", secret.display());
        let cmd = format!("powershell -enc {}", encode_ps(&script));
        assert!(command_secret(&cmd, &f.work, &f.data).is_some(), "{cmd}");
    }

    #[test]
    fn encoded_ec_alias_and_inline_value_keep_their_case() {
        let f = fixture("enc-alias");
        let secret = f.data.join("secrets.json");
        let script = format!("Get-Content '{}'", secret.display());
        let ec = format!("powershell -ec {}", encode_ps(&script));
        assert!(command_secret(&ec, &f.work, &f.data).is_some(), "{ec}");
        // `-enc:<base64>`: значение не переводится в нижний регистр, безобидная команда проходит.
        let inline = format!("powershell -EncodedCommand:{}", encode_ps("echo hi"));
        assert!(command_secret(&inline, &f.work, &f.data).is_none(), "{inline}");
        let inline_secret = format!("powershell -enc:{}", encode_ps(&script));
        assert!(command_secret(&inline_secret, &f.work, &f.data).is_some(), "{inline_secret}");
    }

    #[test]
    fn encoded_harmless_not_refused() {
        let f = fixture("enc-harmless");
        let cmd = format!("powershell -enc {}", encode_ps("echo hi"));
        assert!(command_secret(&cmd, &f.work, &f.data).is_none(), "{cmd}");
        assert!(!command_needs_approval(&cmd, &f.work, &f.data), "{cmd}");
    }

    #[test]
    fn encoded_write_outside_asks() {
        let f = fixture("enc-write");
        let other = f.other.display().to_string();
        let script = format!("Remove-Item '{}\\readme.md'", other);
        let cmd = format!("powershell -enc {}", encode_ps(&script));
        assert!(command_needs_approval(&cmd, &f.work, &f.data), "{cmd}");
    }

    #[test]
    fn secrets_file_neighbours_are_key_files_outside_the_data_folder() {
        let f = fixture("secrets-neighbours");
        for name in ["secrets.json.v1", "secrets.json.migrating", "secrets.json.tmp", "secrets.json.bad-1760000000"] {
            let copy = f.other.join(name);
            assert_eq!(secret_reason(&copy, &f.data, &f.work), Some(KEY_FILE), "{name}");
        }
    }

    #[test]
    fn encoded_abbreviations_refused() {
        let f = fixture("enc-abbr");
        let secret = f.data.join("secrets.json");
        let script = format!("Get-Content '{}'", secret.display());
        let encoded = encode_ps(&script);
        for flag in ["-e", "-en", "-enc", "-EncodedCommand"] {
            let cmd = format!("powershell {} {}", flag, encoded);
            assert!(command_secret(&cmd, &f.work, &f.data).is_some(), "{flag}: {cmd}");
        }
    }

    #[test]
    fn encoded_bad_base64_refused() {
        let f = fixture("enc-bad");
        let cmd = "powershell -enc !!!not-base64!!!".to_string();
        let (raw, reason) = command_secret(&cmd, &f.work, &f.data).expect("refused");
        let _ = raw;
        assert!(reason.contains("проверить"), "{reason}");
        assert!(command_needs_approval(&cmd, &f.work, &f.data), "approval");
    }

    #[test]
    fn encoded_nested_depth2_caught() {
        let f = fixture("enc-nest2");
        let secret = f.data.join("secrets.json");
        let inner_script = format!("Get-Content '{}'", secret.display());
        let inner_encoded = encode_ps(&inner_script);
        let outer_script = format!("powershell -enc {}", inner_encoded);
        let outer_encoded = encode_ps(&outer_script);
        let cmd = format!("powershell -enc {}", outer_encoded);
        assert!(command_secret(&cmd, &f.work, &f.data).is_some(), "{cmd}");
    }

    #[test]
    fn encoded_depth4_refused() {
        let f = fixture("enc-depth4");
        let secret = f.data.join("secrets.json");
        // 4 levels of encoding around a secret read.
        let mut script = format!("Get-Content '{}'", secret.display());
        for _ in 0..4 {
            script = format!("powershell -enc {}", encode_ps(&script));
        }
        assert!(command_secret(&script, &f.work, &f.data).is_some(), "{script}");
    }
}

#[cfg(test)]
mod guard_list_tests {
    use super::*;

    #[test]
    fn a_folder_that_looks_like_a_bare_repository_is_guarded_without_a_dot_git_name() {
        let dir = std::env::temp_dir().join(format!("deloist-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("objects")).unwrap();
        std::fs::create_dir_all(dir.join("hooks")).unwrap();
        std::fs::write(dir.join("HEAD"), "ref: refs/heads/main").unwrap();
        assert!(auto_exec_reason(&dir.join("hooks").join("pre-receive")).is_some());
        assert!(auto_exec_reason(&dir.join("config")).is_some());
        assert!(auto_exec_reason(&dir.join("notes.md")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn runnable_files_are_recognized_by_extension_and_shell_names() {
        for name in ["git.exe", "a.bat", "b.cmd", "c.ps1", "d.lnk", "e.vbs", "f.js", "g.wsf", "h.dll", "pwsh", "cmd.exe", "powershell.bat", "i.hta", "j.jar", "k.url"] {
            assert!(planted_executable(Path::new(name)), "{name}");
        }
        for name in ["notes.md", "a.py", "data.json", "page.html", "readme.txt", "x.rs", "cmdline.md"] {
            assert!(!planted_executable(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn autorun_and_agent_files_are_guarded_inside_projects() {
        for path in [
            "C:/p/.git/hooks/pre-commit", "C:/p/.git/config", "C:/p/.gitattributes", "C:/p/.claude/settings.json", "C:/p/.claude/settings.local.json",
            "C:/p/.claude/hooks/a.cmd", "C:/p/.claude/commands/x.md", "C:/p/.claude/agents/x.md", "C:/p/.mcp.json", "C:/p/.codex/config.toml",
            "C:/p/.gemini/settings.json", "C:/p/.cursor/rules/a.mdc", "C:/p/.npmrc", "C:/p/.yarnrc.yml", "C:/p/.gitmodules", "C:/p/.github/workflows/ci.yml",
            "C:/p/.pre-commit-config.yaml", "C:/p/package.json", "C:/p/crate/build.rs", "C:/p/Makefile", "C:/p/.cargo/config.toml", "C:/p/.vscode/tasks.json",
            "C:/p/.vscode/launch.json", "C:/p/.envrc", "C:/p/Microsoft.PowerShell_profile.ps1", "C:/p/.husky/pre-push",
        ] {
            assert!(auto_exec_reason(Path::new(path)).is_some(), "{path}");
            assert!(project_guard_reason(Path::new(path)).is_some(), "{path}");
        }
        // Ещё агенты и редакторы, голые репозитории.
        for path in [
            "C:/p/.claude/skills/x/SKILL.md", "C:/p/opencode.json", "C:/p/.opencode/agent/a.md", "C:/p/.windsurf/rules.md", "C:/p/.aider.conf.yml",
            "C:/p/.continue/config.json", "C:/p/repo.git/hooks/post-receive", "C:/p/repo.git/config",
        ] {
            assert!(auto_exec_reason(Path::new(path)).is_some(), "{path}");
        }
        // Документы для агентов и сценарии `tools/`, `scripts/` закрыты только в папках проектов и команд.
        for path in ["C:/p/AGENTS.md", "C:/p/CLAUDE.md", "C:/p/tools/release.ps1", "C:/p/scripts/run.sh", "C:/p/tools/x/a.cmd", "C:/p/scripts/b.bat"] {
            assert!(auto_exec_reason(Path::new(path)).is_none(), "{path}");
            assert!(project_guard_reason(Path::new(path)).is_some(), "{path}");
        }
        // Обычные файлы проекта и Cargo.toml открыты.
        for path in ["C:/p/src/lib.rs", "C:/p/Cargo.toml", "C:/p/README.md", "C:/p/docs/a.md", "C:/p/tools/readme.md", "C:/p/web/app.js"] {
            assert!(project_guard_reason(Path::new(path)).is_none(), "{path}");
        }
    }
}
