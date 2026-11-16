//! Ключи проектов: файл `.env` на компьютере или на сервере, ключ SSH.
//!
//! Бот не видит значений. Прочитать файл с ключами можно только с карточки `secret` (один раз
//! или на этот разговор, никогда «всегда»). Deloist сам читает файл, кладёт значения в хранилище
//! секретов (DPAPI, область `bot:<id>`) и отдаёт модели `ИМЯ={{secret:ИМЯ}}`. Дальше бот пишет
//! `{{secret:ИМЯ}}` в команде: значение уходит процессу переменной окружения, а в чат, журнал и
//! память попадает только ссылка.

use serde_json::Value;

/// Что трогает вызов: место для разрешения и подпись для карточки.
#[derive(Clone, Debug, PartialEq)]
pub struct Touch {
    /// Ключ разрешения: `myserver:.env`, `local:.env`, путь файла.
    pub place: String,
    /// «.env на myserver».
    pub label: String,
    /// Ключ SSH, а не файл с ключами.
    pub ssh_key: bool,
}

impl Touch {
    pub fn title(&self) -> String {
        if self.ssh_key {
            format!("Бот хочет использовать ключ SSH: {}", self.label)
        } else {
            format!("Бот хочет прочитать файл с ключами: {}", self.label)
        }
    }

    pub fn scope(&self, tool: &str) -> crate::grants::Scope {
        crate::grants::Scope::new(tool, &self.place, KIND, "secret", &self.label).conversation_only()
    }
}

/// Вид разрешения: только «один раз» или «в этом разговоре».
pub const KIND: &str = "secret";

/// Похоже ли имя файла на файл с ключами.
pub fn is_key_file_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    if name == ".env" {
        return true;
    }
    if let Some(rest) = name.strip_prefix(".env.") {
        // Образцы без значений: их читать можно.
        return !matches!(rest, "example" | "sample" | "template" | "dist" | "defaults");
    }
    // `secrets.json` и его соседи: `.migrating`, `.tmp`, `.bad-*`, `.v1`.
    name.starts_with("secrets.json")
        || matches!(name.as_str(), "credentials.json" | "secrets.key" | "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".ppk")
}

fn file_part(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

fn tokens(command: &str) -> Vec<String> {
    command
        .split(|c: char| c.is_whitespace() || "\"'`;|&<>(),".contains(c))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Ключи ssh/scp, после которых идёт значение.
const SSH_VALUE_FLAGS: &[&str] =
    &["-i", "-o", "-p", "-P", "-l", "-F", "-J", "-L", "-R", "-D", "-c", "-m", "-b", "-E", "-S", "-W", "-Q", "-e", "-O", "-B"];

/// Сервер, на который идёт команда ssh/scp/sftp: псевдоним или адрес без пользователя.
pub fn remote_host(command: &str) -> Option<String> {
    let words = tokens(command);
    let at = words.iter().position(|w| {
        let name = file_part(w).to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        matches!(name, "ssh" | "scp" | "sftp")
    })?;
    let mut rest = words[at + 1..].iter();
    while let Some(word) = rest.next() {
        if word.starts_with('-') {
            if SSH_VALUE_FLAGS.contains(&word.as_str()) {
                rest.next();
            }
            continue;
        }
        let host = word.rsplit('@').next().unwrap_or(word);
        let host = match host.split_once(':') {
            // `C:\…` у scp — локальный путь, сервер дальше.
            Some((drive, _)) if drive.len() == 1 => continue,
            Some((host, _)) => host,
            None => host,
        };
        if !host.is_empty() {
            return Some(host.to_string());
        }
    }
    None
}

/// Трогает ли команда файл с ключами или ключ SSH. `None` — нет.
pub fn command_touch(command: &str) -> Option<Touch> {
    let words = tokens(command);
    let host = remote_host(command);
    let at = |file: &str| match &host {
        Some(host) => (format!("{host}:{file}"), format!("{file} на {host}")),
        None => (format!("local:{file}"), file.to_string()),
    };
    // Ключ SSH в явном виде: `ssh -i C:\keys\vps` (у `grep -i` и `sed -i` сервера нет).
    if let Some(i) = words.iter().position(|w| w == "-i").filter(|_| host.is_some()) {
        if let Some(path) = words.get(i + 1) {
            let name = file_part(path).to_string();
            return Some(Touch { place: format!("ssh-key:{}", path.to_lowercase()), label: name, ssh_key: true });
        }
    }
    for word in &words {
        let name = file_part(word.rsplit(':').next().unwrap_or(word));
        // Мастер-ключ Deloist закрыт наглухо (sandbox отказывает по тексту), карточки на него нет.
        if name.eq_ignore_ascii_case("secrets.key") {
            continue;
        }
        if is_key_file_name(name) {
            let (place, label) = at(&name.to_lowercase());
            return Some(Touch { place, label, ssh_key: false });
        }
    }
    // `python -c "…load_dotenv()…"` читает `.env` из текущей папки, имени в тексте нет.
    let lower = command.to_lowercase();
    if lower.contains("dotenv") || lower.contains("env_file") {
        let (place, label) = at(".env");
        return Some(Touch { place, label, ssh_key: false });
    }
    None
}

/// Трогает ли путь (из инструмента файлов или из запроса агента) файл с ключами.
pub fn path_touch(path: &str) -> Option<Touch> {
    let name = file_part(path);
    if !is_key_file_name(name) || name.eq_ignore_ascii_case("secrets.key") {
        return None;
    }
    // Ключи SSH, облаков и самой программы закрыты всегда: карточки нет, инструмент откажет сам.
    let closed = [".ssh", ".aws", ".gnupg", ".azure", ".kube", ".docker", ".deloist"];
    if path.split(['/', '\\']).any(|part| closed.iter().any(|c| part.eq_ignore_ascii_case(c))) {
        return None;
    }
    let shown = crate::reach::display(std::path::Path::new(path));
    let folder = std::path::Path::new(&shown).parent().map(|p| p.display().to_string()).unwrap_or_default();
    let label = if folder.is_empty() { name.to_string() } else { format!("{name} в {folder}") };
    Some(Touch { place: shown.to_lowercase(), label, ssh_key: false })
}

/// Что трогает вызов инструмента бота.
pub fn call_touch(name: &str, args: &Value) -> Option<Touch> {
    let arg = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
    match name {
        "run_command" => command_touch(arg("command")),
        "read_file" if !is_ssh_config(arg("path")) => path_touch(arg("path")),
        _ => None,
    }
}

/// `~/.ssh/config`: из него бот видит только имена хостов.
pub fn is_ssh_config(path: &str) -> bool {
    let parts: Vec<String> = path.split(['/', '\\']).filter(|p| !p.is_empty()).map(str::to_lowercase).collect();
    parts.len() >= 2 && parts[parts.len() - 1] == "config" && parts[parts.len() - 2] == ".ssh"
}

/// Строки `Host` из `~/.ssh/config`, без адресов, пользователей и путей к ключам.
pub fn ssh_config_hosts(text: &str) -> String {
    let hosts: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, rest) = line.split_once(char::is_whitespace)?;
            key.eq_ignore_ascii_case("host").then(|| rest.trim().to_string())
        })
        .filter(|h| !h.is_empty())
        .collect();
    if hosts.is_empty() {
        return "В ~/.ssh/config нет записей Host.".into();
    }
    format!(
        "Псевдонимы серверов из ~/.ssh/config (остальное в файле закрыто): {}\nПодключайся так: ssh <псевдоним> <команда>, без -i.",
        hosts.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Значения
// ---------------------------------------------------------------------------

fn unquote(value: &str) -> String {
    let v = value.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return v[1..v.len() - 1].to_string();
        }
    }
    // Комментарий после значения без кавычек.
    match v.find(" #") {
        Some(at) => v[..at].trim_end().to_string(),
        None => v.to_string(),
    }
}

/// Строки `ИМЯ=значение` заменяются на `ИМЯ={{secret:ИМЯ}}`; найденные пары возвращаются.
pub fn redact_pairs(text: &str) -> (String, Vec<(String, String)>) {
    let mut found = Vec::new();
    let mut out = Vec::new();
    for line in text.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix("\r\n") {
            Some(b) => (b, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (line, ""),
            },
        };
        let trimmed = body.trim_start();
        let indent = &body[..body.len() - trimmed.len()];
        let (export, assignment) = match trimmed.strip_prefix("export ") {
            Some(rest) => ("export ", rest.trim_start()),
            None => ("", trimmed),
        };
        let parsed = assignment.split_once('=').and_then(|(name, value)| {
            let name = name.trim();
            let ok = !name.is_empty()
                && !name.starts_with('#')
                && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && crate::secrets::valid_name(name);
            let value = unquote(value);
            (ok && !value.is_empty()).then(|| (name.to_string(), value))
        });
        match parsed {
            Some((name, value)) => {
                out.push(format!("{indent}{export}{name}={{{{secret:{name}}}}}{end}"));
                found.push((name, value));
            }
            None => out.push(line.to_string()),
        }
    }
    (out.concat(), found)
}

/// Убрать из текста известные значения: каждое становится ссылкой `{{secret:ИМЯ}}`.
pub fn scrub(text: &str, known: &[(String, String)]) -> String {
    let mut pairs: Vec<&(String, String)> = known.iter().filter(|(_, v)| v.chars().count() >= 4).collect();
    pairs.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
    let mut out = text.to_string();
    for (name, value) in pairs {
        if out.contains(value.as_str()) {
            out = out.replace(value.as_str(), &format!("{{{{secret:{name}}}}}"));
        }
    }
    out
}

/// Имя переменной окружения для ключа.
pub fn env_name(name: &str) -> String {
    let body: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    format!("DELOIST_SECRET_{body}")
}

/// Команда с `{{secret:ИМЯ}}` → команда с `%DELOIST_SECRET_ИМЯ%` и переменные для процесса.
/// Оболочка — `cmd /c`: она раскрывает `%…%` раньше, чем стартуют powershell, curl или ssh.
pub fn prepare_command(command: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<(String, Vec<(String, String)>), String> {
    let names = crate::secrets::names_in(command);
    let mut env = Vec::new();
    for name in &names {
        if !crate::secrets::valid_name(name) {
            return Err(format!("недопустимое имя ключа «{name}»"));
        }
        match lookup(name) {
            Some(value) if !value.is_empty() => env.push((env_name(name), value)),
            _ => {
                return Err(format!(
                    "нет ключа {name}: сначала прочитай файл с ключами (Deloist покажет карточку) или попроси человека"
                ))
            }
        }
    }
    let prepared = crate::secrets::substitute(command, |name| Some(format!("%{}%", env_name(name))))?;
    Ok((prepared, env))
}

// ---------------------------------------------------------------------------
// Хранилище
// ---------------------------------------------------------------------------

/// Область бота в хранилище секретов. Id MCP-сервера не содержит `:`, так что с подключением не совпадёт.
pub fn scope(bot_id: &str) -> String {
    format!("bot:{bot_id}")
}

/// Ключи бота: имя и значение. Только для подстановки и вычистки, наружу не отдаются.
pub fn known(app: &crate::App, bot_id: &str) -> Vec<(String, String)> {
    let store = &app.connectors.secrets;
    let scope = scope(bot_id);
    store.names(&scope).into_iter().filter_map(|n| store.get(&scope, &n).map(|v| (n, v))).collect()
}

/// Сохранить найденные ключи бота.
pub fn save(app: &crate::App, bot_id: &str, pairs: &[(String, String)]) -> Result<(), String> {
    let scope = scope(bot_id);
    for (name, value) in pairs {
        app.connectors.secrets.set(&scope, name, value)?;
    }
    Ok(())
}

/// Вывод, прочитанный с разрешения человека: значения — в хранилище, модели — ссылки.
pub fn take_values(app: &crate::App, bot_id: &str, text: &str) -> Result<(String, usize), String> {
    let (redacted, pairs) = redact_pairs(text);
    save(app, bot_id, &pairs)?;
    Ok((scrub(&redacted, &known(app, bot_id)), pairs.len()))
}

/// Подсказка модели после чтения ключей.
pub fn saved_note(count: usize) -> String {
    if count == 0 {
        "Ключей вида ИМЯ=значение не нашлось.".into()
    } else {
        format!(
            "Deloist сохранил ключей: {count}. Значения тебе не показываются: пиши в run_command ссылку {{{{secret:ИМЯ}}}}, например curl -H \"Api-Key: {{{{secret:ИМЯ}}}}\" …; Deloist подставит значение сам."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_files_are_recognised_and_samples_are_not() {
        for name in [".env", ".env.local", ".ENV.production", "secrets.json", "secrets.json.v1", "secrets.json.migrating", "secrets.json.tmp", "secrets.json.bad-1760000000", "credentials.json", "server.pem", "api.key", "id_ed25519"] {
            assert!(is_key_file_name(name), "{name}");
        }
        for name in [".env.example", ".env.sample", "env.md", "README.md", "id_ed25519.pub", "config.json", ".envrc.md"] {
            assert!(!is_key_file_name(name), "{name}");
        }
    }

    #[test]
    fn commands_that_reach_key_files_are_caught() {
        let t = command_touch("ssh myserver cat /home/user/proj/.env").unwrap();
        assert_eq!((t.place.as_str(), t.label.as_str()), ("myserver:.env", ".env на myserver"));
        assert_eq!(t.title(), "Бот хочет прочитать файл с ключами: .env на myserver");
        let t = command_touch("ssh -o BatchMode=yes user@203.0.113.10 \"cd /home/user/proj && python3 -c 'from dotenv import load_dotenv; load_dotenv()'\"").unwrap();
        assert_eq!(t.place, "203.0.113.10:.env");
        let t = command_touch(r"scp myserver:/srv/app/.env.production C:\tmp\x").unwrap();
        assert_eq!(t.label, ".env.production на myserver");
        let t = command_touch(r"type ..\..\proj\.env").unwrap();
        assert_eq!(t.place, "local:.env");
        let t = command_touch(r"ssh -i C:\Users\me\.ssh\id_test user@1.2.3.4 uptime").unwrap();
        assert!(t.ssh_key && t.title().contains("id_test"));
        for plain in ["ssh myserver uptime", "git status", "curl -H \"Api-Key: {{secret:KEY}}\" https://api-seller.ozon.ru", "type README.md", "copy .env.example x.txt"] {
            assert_eq!(command_touch(plain), None, "{plain}");
        }
        assert_eq!(remote_host("ssh -p 22 -l root myserver ls").as_deref(), Some("myserver"));
        assert!(path_touch(r"C:\proj\.env").is_some());
        assert_eq!(path_touch(r"C:\Users\me\.ssh\id_ed25519"), None, "ключи SSH закрыты без карточки");
    }

    #[test]
    fn ssh_config_shows_host_names_only() {
        let text = "Host myserver\n  HostName 203.0.113.10\n  User deploy\n  IdentityFile C:\\Users\\me\\.ssh\\vps\nHost github.com\n  IdentityFile x\n";
        let shown = ssh_config_hosts(text);
        assert!(shown.contains("myserver, github.com"), "{shown}");
        for hidden in ["216.57", "codex\n", "IdentityFile", "vps"] {
            assert!(!shown.contains(hidden), "{hidden} в {shown}");
        }
        assert!(is_ssh_config(r"C:\Users\me\.ssh\config") && is_ssh_config("~/.ssh/config") && !is_ssh_config(r"C:\proj\config"));
    }

    #[test]
    fn env_values_become_references() {
        let text = "# Ozon\r\nOZON_CLIENT_ID=131000\r\nexport OZON_SELLER_API_KEY=\"0f0e0d0c-1111-2222-3333-444455556666\"\r\nEMPTY=\r\nDEBUG=1 # dev\r\n";
        let (shown, pairs) = redact_pairs(text);
        assert_eq!(shown, "# Ozon\r\nOZON_CLIENT_ID={{secret:OZON_CLIENT_ID}}\r\nexport OZON_SELLER_API_KEY={{secret:OZON_SELLER_API_KEY}}\r\nEMPTY=\r\nDEBUG={{secret:DEBUG}}\r\n");
        assert_eq!(pairs[1], ("OZON_SELLER_API_KEY".to_string(), "0f0e0d0c-1111-2222-3333-444455556666".to_string()));
        assert_eq!(pairs[2].1, "1");
        let out = scrub("HTTP 200, key 0f0e0d0c-1111-2222-3333-444455556666 ok", &pairs);
        assert_eq!(out, "HTTP 200, key {{secret:OZON_SELLER_API_KEY}} ok");
        assert_eq!(scrub("a 1 b", &pairs), "a 1 b", "короткие значения не трогаем");
    }

    #[test]
    fn references_become_environment_variables() {
        let lookup = |n: &str| (n == "OZON_SELLER_API_KEY").then(|| "v4lue".to_string());
        let (cmd, env) = prepare_command("curl -H \"Api-Key: {{secret:OZON_SELLER_API_KEY}}\" https://x", lookup).unwrap();
        assert_eq!(cmd, "curl -H \"Api-Key: %DELOIST_SECRET_OZON_SELLER_API_KEY%\" https://x");
        assert_eq!(env, vec![("DELOIST_SECRET_OZON_SELLER_API_KEY".to_string(), "v4lue".to_string())]);
        let err = prepare_command("echo {{secret:NOPE}}", lookup).unwrap_err();
        assert!(err.contains("нет ключа NOPE") && !err.contains("v4lue"));
        assert_eq!(prepare_command("echo hi", lookup).unwrap(), ("echo hi".to_string(), vec![]));
    }
}
