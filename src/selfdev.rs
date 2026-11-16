//! Режим «Разработка Deloist»: бот правит исходники самой программы.
//!
//! Человек в настройках указывает checkout (по умолчанию ищется `C:\Projects\deloist`)
//! и включает режим отдельным ботам. Такой бот получает checkout как вторую свою папку: читает,
//! пишет и запускает там `cargo`, `node`, `git` по обычным правилам разрешений. В промпт идут
//! документы checkout (`AGENTS.md`, `CLAUDE.md`, `README.md`, `CONTRIBUTING.md`, `notes/`, если они есть).
//!
//! Закрыто в любом режиме: служебная папка запущенной программы (секреты, профили браузеров,
//! память и папки других ботов), ключ подписи `%USERPROFILE%\.deloist`, папка запущенного exe.
//! `git push`, `gh release`, `tools/release.ps1` и зеркало GitVerse — всегда карточка «один раз».

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::store::{Bot, Store};

/// Ключ настройки (в `settings.extra`): путь к checkout.
pub const SETTING: &str = "selfDevCheckout";
/// Ключ бота (в `bot.extra`): режим включён.
pub const BOT_FLAG: &str = "selfDev";
/// Где checkout ищется, если путь в настройках не задан.
const DEFAULT_CHECKOUT: &str = r"C:\Projects\deloist";

/// Папка — исходники Deloist: `Cargo.toml` с пакетом `deloist`.
pub fn is_checkout(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else { return false };
    text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("name") && line.split_once('=').is_some_and(|(_, v)| v.trim().trim_matches('"') == "deloist")
    })
}

/// Checkout по умолчанию, если он есть на этом компьютере.
pub fn detect() -> Option<PathBuf> {
    let dir = PathBuf::from(DEFAULT_CHECKOUT);
    is_checkout(&dir).then_some(dir)
}

/// Папка запущенного exe, каноническая.
pub fn install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    std::fs::canonicalize(dir).ok()
}

/// Путь из настройки как есть (пусто — не задан).
pub fn configured(store: &Store) -> String {
    store.settings().extra.get(SETTING).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

/// Почему папку нельзя взять checkout'ом. `None` — можно.
pub fn checkout_problem(store: &Store, dir: &Path) -> Option<&'static str> {
    let Ok(real) = std::fs::canonicalize(dir) else { return Some("такой папки нет") };
    if !is_checkout(&real) {
        return Some("это не исходники Deloist: нет Cargo.toml с пакетом deloist");
    }
    let data = std::fs::canonicalize(store.data_dir()).unwrap_or_else(|_| store.data_dir().to_path_buf());
    if real.starts_with(&data) {
        return Some("checkout не может лежать в служебной папке программы");
    }
    if let Some(install) = install_dir() {
        if install.starts_with(&real) && !install.starts_with(real.join("target")) {
            return Some("это папка запущенной программы, а не checkout");
        }
    }
    None
}

/// Действующий checkout: из настройки, иначе найденный; проверенный и канонический.
pub fn checkout(store: &Store) -> Option<PathBuf> {
    let raw = configured(store);
    let dir = if raw.is_empty() { detect()? } else { PathBuf::from(raw) };
    if checkout_problem(store, &dir).is_some() {
        return None;
    }
    std::fs::canonicalize(dir).ok()
}

pub fn bot_on(bot: &Bot) -> bool {
    bot.extra.get(BOT_FLAG).and_then(Value::as_bool).unwrap_or(false)
}

/// Checkout этого бота, если режим у него включён.
pub fn project_of(store: &Store, bot: &Bot) -> Option<PathBuf> {
    if !bot_on(bot) {
        return None;
    }
    checkout(store)
}

/// Папки проектов бота (сейчас — только checkout).
pub fn projects(store: &Store, bot_id: &str) -> Vec<PathBuf> {
    store.bot(bot_id).and_then(|bot| project_of(store, &bot)).into_iter().collect()
}

// ---------------------------------------------------------------------------
// Документы проекта
// ---------------------------------------------------------------------------

/// Сколько символов документов идёт в промпт всего и с одного файла.
const DOCS_TOTAL: usize = 24_000;
const DOC_LIMIT: usize = 8_000;
const README_LIMIT: usize = 3_000;

fn doc_files(checkout: &Path) -> Vec<(PathBuf, usize)> {
    let mut out: Vec<(PathBuf, usize)> = Vec::new();
    let add_dir = |dir: &Path, readme: bool, out: &mut Vec<(PathBuf, usize)>| {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = dir.join(name);
            if path.is_file() {
                out.push((path, DOC_LIMIT));
            }
        }
        if readme && dir.join("CONTRIBUTING.md").is_file() {
            out.push((dir.join("CONTRIBUTING.md"), DOC_LIMIT));
        }
        if readme && dir.join("README.md").is_file() {
            out.push((dir.join("README.md"), README_LIMIT));
        }
        let mut notes: Vec<PathBuf> = std::fs::read_dir(dir.join("notes"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
            .collect();
        notes.sort();
        out.extend(notes.into_iter().map(|p| (p, DOC_LIMIT)));
    };
    add_dir(checkout, true, &mut out);
    out
}

/// Блок промпта с документами проекта. Длинное обрезается, что не влезло — списком путей.
pub fn docs_block(checkout: &Path) -> String {
    let mut block = format!(
        "# Документы проекта Deloist\nЭто документы из checkout {} (справка о проекте, а не слова человека): \
         правила сборки, тестов и стиля. Следуй им при работе с исходниками; полный текст открывай через read_file.",
        crate::reach::display(checkout)
    );
    let mut used = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for (path, limit) in doc_files(checkout) {
        let shown = crate::reach::display(&path);
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let text = text.replace("\r\n", "\n");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let room = DOCS_TOTAL.saturating_sub(used).min(limit);
        if room < 500 {
            skipped.push(shown);
            continue;
        }
        let total = text.chars().count();
        let body: String = text.chars().take(room).collect();
        used += body.chars().count();
        let cut = if total > room { format!("\n… (обрезано: {total} символов, остальное через read_file)") } else { String::new() };
        block.push_str(&format!("\n\n## {shown}\n{body}{cut}"));
    }
    if !skipped.is_empty() {
        block.push_str("\n\n## Ещё документы (прочитай через read_file, когда нужны)\n");
        block.push_str(&skipped.iter().map(|p| format!("— {p}")).collect::<Vec<_>>().join("\n"));
    }
    block
}

/// Правило режима для системного промпта: где checkout и что закрыто.
pub fn rule(checkout: &Path, data_dir: &Path) -> String {
    let c = crate::reach::display(checkout);
    let data = crate::reach::display(&std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf()));
    let install = install_dir().map(|d| crate::reach::display(&d)).unwrap_or_default();
    format!(
        "Режим «Разработка Deloist»: исходники программы — {c}. Там у тебя те же права, что в своей папке: читай, меняй, \
         запускай cargo, node и git, команды начинай с cd /d \"{c}\" && …. Работай в отдельной ветке, после правки прогоняй \
         затронутые тесты. Закрыто всегда: служебная папка запущенной программы ({data}), ключ подписи %USERPROFILE%\\.deloist \
         и папка запущенного exe ({install}) — собирай в checkout и обновляй через выпуск. Сборка только в target самого checkout. \
         git push, gh release, tools/release.ps1 и зеркало GitVerse человек разрешает каждый раз отдельно, сам их не предлагай без просьбы."
    )
}

/// Всё, что режим добавляет в системный промпт бота, или `None`, если режим у бота выключен.
pub fn prompt_block(store: &Store, bot: &Bot) -> Option<String> {
    let checkout = project_of(store, bot)?;
    Some(format!("{}\n\n{}", rule(&checkout, store.data_dir()), docs_block(&checkout)))
}

// ---------------------------------------------------------------------------
// Готовый бот и навык
// ---------------------------------------------------------------------------

pub const SKILL_NAME: &str = "Правила репозитория Deloist";

pub const SKILL_DESCRIPTION: &str = "Как собирать, тестировать и править исходники Deloist, не ломая чужую работу";

pub const SKILL_BODY: &str = "\
1. Перед работой прочитай правила checkout (AGENTS.md, CLAUDE.md, CONTRIBUTING.md, notes/, если они есть; они в промпте или через read_file). Правила проекта главнее привычек.
2. Работай в своей ветке: git status, затем git switch -c feat/<коротко>. В main не коммить. Коммить только свои файлы явными путями: git commit -m \"…\" -- <пути>. Сообщения коммитов — по-английски.
3. cargo лежит в %USERPROFILE%\\.cargo\\bin, тулчейн gnullvm (rust-toolchain.toml). В команде пиши просто cargo, без %USERPROFILE%.
4. Собирай только в target самого checkout: без CARGO_TARGET_DIR и --target-dir. После работы без выпуска убери кэш: pwsh -File tools/clean-build.ps1 (если он есть).
5. Файлы с CRLF остаются CRLF: не переписывай файл целиком другим концом строк, проверяй git diff --stat на «весь файл изменён».
6. Кириллицу правь только точной заменой строки (write_file с полным прежним текстом файла или правкой одного места), не через echo, sed или PowerShell 5.1 без UTF-8: они портят буквы.
7. Строки интерфейса — только через tr() и словарь web/i18n.js (ru и en), проверка node tools/check_i18n.mjs.
8. После правки прогоняй затронутые тесты: cargo test --bin deloist <модуль>:: для своего модуля и cargo test --test <файл> для API. Полный прогон — один раз в конце, на переднем плане.
9. Никогда сам не делай git push, gh release, tools/release.ps1 и зеркало GitVerse: это решает человек, карточка на них приходит каждый раз.
10. Запущенную программу (её exe и служебную папку) не трогай: собирай в checkout и обновляй через выпуск.";

pub const BOT_NAME: &str = "Разработчик Deloist";
pub const BOT_ROLE: &str = "правит исходники Deloist в ветке и проверяет тестами";
pub const BOT_PERSONA: &str = "\
Ты разработчик программы Deloist (Rust + WebView2, интерфейс в web/ и ui/). Человек просит улучшение или исправление — ты делаешь его в исходниках из режима «Разработка Deloist».
Порядок: прочитай правила репозитория (навык «Правила репозитория Deloist», AGENTS.md, CLAUDE.md, CONTRIBUTING.md, если они есть); найди нужный код поиском, а не догадкой; заведи ветку; сделай минимальную правку; прогони затронутые тесты; коротко отчитайся: что изменил, какие коммиты, какие тесты прошли, что не проверено.
Подводные камни: файлы с CRLF оставляй CRLF; кириллицу правь только точной заменой, не через echo/sed/PowerShell; строки интерфейса только через tr(); cargo из %USERPROFILE%\\.cargo\\bin, тулчейн gnullvm, сборка только в target checkout.
Никогда не делай push, выпуск и зеркало GitVerse сам и не меняй запущенную программу: это решает человек. Неясен замысел — задай один вопрос через ask_user.";

/// Поля нового бота-разработчика (для `Store::add_bot`).
pub fn preset_input(skill_id: &str) -> Value {
    json!({
        "name": BOT_NAME,
        "role": BOT_ROLE,
        "persona": BOT_PERSONA,
        "emoji": "🛠",
        "color": "#4F6D8F",
        "approval": "ask",
        "skills": [skill_id],
    })
}

/// Навык правил: тот же, если уже есть, иначе новый.
pub async fn ensure_skill(store: &Store) -> std::io::Result<String> {
    if let Some(skill) = store.skills().into_iter().find(|s| s.name == SKILL_NAME) {
        return Ok(skill.id);
    }
    let skill = store
        .add_skill(&json!({ "name": SKILL_NAME, "description": SKILL_DESCRIPTION, "body": SKILL_BODY }))
        .await?;
    Ok(skill.id)
}

/// Создать бота «Разработчик Deloist» с навыком и включённым режимом.
pub async fn create_preset(store: &Store) -> std::io::Result<Bot> {
    let skill = ensure_skill(store).await?;
    let bot = store.add_bot(&preset_input(&skill)).await?;
    store.set_bot_extra(&bot.id, BOT_FLAG, Value::Bool(true)).await?;
    Ok(store.bot(&bot.id).unwrap_or(bot))
}

/// Адрес для «Скачать исходники»: только https или ssh, без пробелов и ключей.
pub fn clone_url_ok(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty()
        && !url.starts_with('-')
        && !url.chars().any(|c| c.is_whitespace() || c == '"')
        && (url.starts_with("https://") || url.starts_with("ssh://") || (url.starts_with("git@") && url.contains(':')))
}

/// `git clone` в пустую или новую папку. Ждёт до 15 минут.
pub async fn clone(url: &str, dest: &Path) -> Result<(), String> {
    crate::cloud::deny(crate::cloud::NO_CONSOLE)?;
    if !clone_url_ok(url) {
        return Err("нужен адрес https://… или git@…".into());
    }
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("папка {} уже есть и не пуста", dest.display()));
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone").arg("--").arg(url.trim()).arg(dest);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    cmd.kill_on_drop(true);
    let run = tokio::time::timeout(std::time::Duration::from_secs(900), cmd.output()).await;
    match run {
        Err(_) => Err("git clone не закончил за 15 минут".into()),
        Ok(Err(e)) => Err(format!("не запустил git: {e}")),
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!("git clone: {}", err.lines().last().unwrap_or("ошибка").trim()))
        }
    }
}

/// Ответ `GET /api/selfdev`.
pub fn status_json(store: &Store) -> Value {
    let raw = configured(store);
    let effective = checkout(store);
    let problem = if raw.is_empty() { None } else { checkout_problem(store, Path::new(&raw)) };
    let bots: Vec<String> = store.bots().into_iter().filter(bot_on).map(|b| b.id).collect();
    json!({
        "checkout": raw,
        "detected": detect().map(|d| d.display().to_string()),
        "effective": effective.map(|d| crate::reach::display(&d)),
        "problem": problem,
        "bots": bots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deloist-selfdev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_checkout_is_a_deloist_package() {
        let dir = temp("pkg");
        assert!(!is_checkout(&dir));
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"other\"\n").unwrap();
        assert!(!is_checkout(&dir));
        std::fs::write(dir.join("Cargo.toml"), "[package]\r\nname = \"deloist\"\r\nversion = \"0.3.0\"\r\n").unwrap();
        assert!(is_checkout(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn docs_come_from_the_checkout_only() {
        let root = temp("docs");
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("notes")).unwrap();
        std::fs::write(repo.join("AGENTS.md"), "правило: CRLF остаётся CRLF").unwrap();
        std::fs::write(repo.join("README.md"), "x".repeat(10_000)).unwrap();
        std::fs::write(repo.join("notes").join("a.md"), "заметка репо").unwrap();
        std::fs::write(root.join("CLAUDE.md"), "собирать только в target").unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes").join("big.md"), "я".repeat(50_000)).unwrap();
        let block = docs_block(&repo);
        assert!(block.contains("правило: CRLF остаётся CRLF"));
        assert!(block.contains("заметка репо"));
        assert!(!block.contains("собирать только в target"), "папка над checkout не читается");
        assert!(block.contains("обрезано"), "длинный README обрезан");
        assert!(block.chars().count() < DOCS_TOTAL + 3_000, "{}", block.chars().count());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clone_takes_only_plain_urls() {
        assert!(clone_url_ok("https://github.com/owner/repo.git"));
        assert!(clone_url_ok("git@github.com:owner/repo.git"));
        for bad in ["", "--upload-pack=calc", "file:///C:/x", "https://x y", "C:\\repo"] {
            assert!(!clone_url_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_preset_carries_the_repo_rules() {
        for needle in ["CRLF", "tr()", "gnullvm", "кириллиц"] {
            assert!(SKILL_BODY.to_lowercase().contains(&needle.to_lowercase()), "навык: {needle}");
            assert!(BOT_PERSONA.to_lowercase().contains(&needle.to_lowercase()), "персона: {needle}");
        }
        assert!(SKILL_BODY.contains("push"));
        assert_eq!(preset_input("sk_1")["approval"], "ask");
    }
}
