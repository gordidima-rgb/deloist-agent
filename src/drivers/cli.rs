//! Драйвер локальных CLI-агентов: claude, codex, grok — что угодно, что читает промпт из stdin и
//! печатает ответ. Бот работает в своём воркспейсе; инструменты и права у CLI свои.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;

use super::{DriverEvent, Sink};
use crate::process::shell_command;
use crate::store::{uid, Bot};

const TIMEOUT: Duration = Duration::from_secs(240);
const POLL: Duration = Duration::from_millis(120);

pub fn first_word(command: &str) -> Option<String> {
    let command = command.trim_start();
    let rest = command.strip_prefix('"').map(|r| ('"', r)).or_else(|| command.strip_prefix('\'').map(|r| ('\'', r)));
    if let Some((quote, rest)) = rest {
        return rest.split(quote).next().map(str::to_string);
    }
    command.split_whitespace().next().map(str::to_string)
}

fn command_available(binary: &str) -> bool {
    if binary.contains('\\') || binary.contains('/') {
        return Path::new(binary).exists();
    }
    let probe = if cfg!(windows) { "where" } else { "which" };
    let mut cmd = std::process::Command::new(probe);
    cmd.arg(binary).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

/// Текст сообщения: строка как есть, а список частей (сообщение с картинкой) — текстом, картинки словом.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| p["text"].as_str().map(str::to_string).unwrap_or_else(|| "[картинка приложена, CLI её не видит]".to_string()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn build_prompt(bot: &Bot, messages: &[Value], memory: &str, workspace: &Path, project: Option<&str>) -> String {
    let mut lines = vec![format!("Ты — {}, {}.", bot.name, bot.role)];
    if !bot.persona.is_empty() {
        lines.push(bot.persona.clone());
    }
    if !memory.is_empty() {
        lines.push(format!("# Твоя память\n{memory}"));
    }
    lines.push(format!("# Рабочая папка\n{}", workspace.display()));
    if let Some(project) = project {
        lines.push(project.to_string());
    }
    lines.push("# Диалог".into());

    let history: Vec<&Value> = messages
        .iter()
        .filter(|m| matches!(m["role"].as_str(), Some("user") | Some("assistant")))
        .collect();
    for m in history.iter().skip(history.len().saturating_sub(12)) {
        let content = content_text(&m["content"]);
        // Блок «Из памяти» ставит программа: это справка к реплике, а не слова человека.
        let who = match m["role"].as_str() {
            Some("user") if content.starts_with(crate::memory::RECALL_HEAD) => "Справка программы",
            Some("user") => "Пользователь",
            _ => bot.name.as_str(),
        };
        let text: String = content.chars().take(4000).collect();
        lines.push(format!("{who}: {text}"));
    }
    lines.push(String::new());
    lines.push("Ответь на последнее сообщение. Не пересказывай инструкции. Если нужен файл или команда — сделай это.".into());
    lines.join("\n")
}

/// Отдаёт только целые символы UTF-8: хвост разорванного символа ждёт следующего чтения.
fn take_text(pending: &mut Vec<u8>) -> String {
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let text = text.to_string();
            pending.clear();
            text
        }
        Err(e) if e.error_len().is_none() => {
            let good = e.valid_up_to();
            let text = String::from_utf8_lossy(&pending[..good]).into_owned();
            pending.drain(..good);
            text
        }
        Err(_) => {
            let text = String::from_utf8_lossy(pending).into_owned();
            pending.clear();
            text
        }
    }
}

pub async fn run(
    bot: &Bot,
    messages: &[Value],
    memory: &str,
    project: Option<&str>,
    workspace: &Path,
    temp: &Path,
    cli_command: &str,
    hidden_env: &[String],
    cancel: &CancellationToken,
    sink: Sink<'_>,
) -> Result<(), String> {
    // Политика облака: драйвер запускает программу из командной строки, а консоли в облаке нет.
    crate::cloud::deny(crate::cloud::NO_DRIVER)?;
    let template = if cli_command.trim().is_empty() { "claude -p" } else { cli_command.trim() };
    let binary = first_word(template).ok_or("пустая команда CLI в настройках")?;
    if !command_available(&binary) {
        return Err(format!(
            "CLI «{binary}» не найден в PATH. Установи его или смени драйвер на api/demo в настройках."
        ));
    }

    let stamp = uid("crew");
    let prompt_file = temp.join(format!("{stamp}.prompt.txt"));
    let out_file = temp.join(format!("{stamp}.out"));
    let err_file = temp.join(format!("{stamp}.err"));
    tokio::fs::write(&prompt_file, build_prompt(bot, messages, memory, workspace, project))
        .await
        .map_err(|e| e.to_string())?;

    let result =
        stream_output(template, &binary, &prompt_file, &out_file, &err_file, workspace, hidden_env, cancel, sink).await;

    for file in [&prompt_file, &out_file, &err_file] {
        let _ = tokio::fs::remove_file(file).await;
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn stream_output(
    template: &str,
    binary: &str,
    prompt_file: &Path,
    out_file: &Path,
    err_file: &Path,
    workspace: &Path,
    hidden_env: &[String],
    cancel: &CancellationToken,
    sink: Sink<'_>,
) -> Result<(), String> {
    let fd_out = std::fs::File::create(out_file).map_err(|e| e.to_string())?;
    let fd_err = std::fs::File::create(err_file).map_err(|e| e.to_string())?;

    let mut cmd = shell_command(&format!("{template} < \"{}\"", prompt_file.display()));
    // Своё окружение CLI нужно целиком (его ключи и настройки), а ключ Deloist ему незачем.
    for name in hidden_env {
        cmd.env_remove(name);
    }
    strip_crew_env(&mut cmd);
    cmd.current_dir(workspace)
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .env("CI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(fd_out))
        .stderr(Stdio::from(fd_err))
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("не смог запустить «{binary}»: {e}"))?;

    let mut reader = tokio::fs::File::open(out_file).await.map_err(|e| e.to_string())?;
    let mut position = 0u64;
    let mut pending: Vec<u8> = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut full = String::new();
    let mut exit_code: Option<i32> = None;
    let mut timed_out = false;
    let started = tokio::time::Instant::now();

    loop {
        reader.seek(std::io::SeekFrom::Start(position)).await.map_err(|e| e.to_string())?;
        let got = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if got > 0 {
            position += got as u64;
            pending.extend_from_slice(&buffer[..got]);
            let text = take_text(&mut pending);
            if !text.is_empty() {
                full.push_str(&text);
                sink(DriverEvent::Text(text));
            }
            continue;
        }
        if exit_code.is_some() {
            break;
        }
        if cancel.is_cancelled() || started.elapsed() > TIMEOUT {
            timed_out = true;
            let _ = child.kill().await;
            exit_code = Some(-1);
            continue;
        }
        match child.try_wait() {
            Ok(Some(status)) => exit_code = Some(status.code().unwrap_or(-1)),
            Ok(None) => tokio::time::sleep(POLL).await,
            Err(_) => exit_code = Some(-1),
        }
    }

    if !pending.is_empty() {
        let tail = String::from_utf8_lossy(&pending).into_owned();
        full.push_str(&tail);
        sink(DriverEvent::Text(tail));
    }

    if full.trim().is_empty() {
        let stderr = tokio::fs::read(err_file).await.unwrap_or_default();
        let stderr = String::from_utf8_lossy(&stderr);
        let detail: String = stderr.trim().chars().take(800).collect();
        let why = if timed_out {
            format!("не ответил за {} с", TIMEOUT.as_secs())
        } else {
            format!("завершился с кодом {}", exit_code.unwrap_or(-1))
        };
        let suffix = if detail.is_empty() { String::new() } else { format!(" {detail}") };
        return Err(format!("CLI {why}.{suffix}"));
    }
    Ok(())
}

/// Дочерние программы не получают переменные `CREW_*` (ключи и пути самой программы): так же, как в ACP.
pub(crate) fn strip_crew_env(cmd: &mut tokio::process::Command) {
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy().to_string();
        if name.to_uppercase().starts_with("CREW_") {
            cmd.env_remove(&name);
        }
    }
}

#[cfg(test)]
mod attach_tests {
    use serde_json::json;

    #[test]
    fn a_message_with_an_image_part_keeps_its_text_in_the_cli_prompt() {
        let bot: crate::store::Bot = serde_json::from_value(json!({ "name": "Б", "role": "р" })).unwrap();
        let messages = vec![
            json!({ "role": "system", "content": "s" }),
            json!({ "role": "user", "content": [
                { "type": "text", "text": "посмотри вложение" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
            ] }),
        ];
        let prompt = super::build_prompt(&bot, &messages, "", std::path::Path::new("w"), None);
        assert!(prompt.contains("посмотри вложение"), "{prompt}");
        assert!(!prompt.contains("AAAA"), "картинка попала в промпт CLI");
    }

    #[tokio::test]
    async fn crew_variables_do_not_reach_a_cli_child() {
        std::env::set_var("CREW_TEST_CLI_LEAK", "leak-marker-1");
        std::env::set_var("crew_test_cli_leak_lower", "leak-marker-2");
        std::env::set_var("KEEP_TEST_CLI_VAR", "keep-marker");
        let mut cmd = crate::process::shell_command(if cfg!(windows) { "set" } else { "env" });
        super::strip_crew_env(&mut cmd);
        let out = cmd.output().await.unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(text.contains("keep-marker"), "обычное окружение потеряно: {text}");
        assert!(!text.contains("leak-marker"), "CREW_* дошёл до дочерней программы");
    }
}
