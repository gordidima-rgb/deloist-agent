//! Выводы рабочих процессов: уведомление, сообщение в тред бота, мессенджеры, файл.
//! Новых получателей здесь нет: только то, что человек уже подключил; файлы — только в своей папке процесса.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::store::now;
use crate::workflows::Node;
use crate::App;

pub const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_PATH_CHARS: usize = 200;

/// Папка файлов процесса.
pub fn files_dir(data_dir: &Path, workflow_id: &str) -> PathBuf {
    data_dir.join("workflows").join("files").join(workflow_id)
}

fn reserved(stem: &str) -> bool {
    let upper = stem.to_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit())
}

/// Безопасное имя одного звена пути: без управляющих знаков, `<>:"|?*`, зарезервированных имён Windows и точки или пробела в конце.
pub fn safe_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("в пути пустое звено или «..»".into());
    }
    if name.chars().any(|c| c.is_control() || "<>:\"|?*/\\".contains(c)) {
        return Err("в пути есть недопустимые знаки".into());
    }
    if name.ends_with('.') || name.ends_with(' ') || name.starts_with(' ') {
        return Err("имя не может начинаться с пробела или кончаться точкой или пробелом".into());
    }
    if reserved(name.split('.').next().unwrap_or("")) {
        return Err("зарезервированное имя Windows".into());
    }
    Ok(())
}

/// Путь внутри папки процесса: только относительный, без `..`, дисков и зарезервированных имён.
pub fn safe_relative(path: &str) -> Result<Vec<String>, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("не указан путь файла".into());
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err("путь длиннее 200 знаков".into());
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return Err("путь должен быть относительным, без диска".into());
    }
    let parts: Vec<String> = path.split(['/', '\\']).map(str::to_string).collect();
    for part in &parts {
        safe_name(part)?;
    }
    Ok(parts)
}

/// Записать файл в папку процесса. Возвращает полный путь.
pub fn write_file(data_dir: &Path, workflow_id: &str, rel: &str, content: &str) -> Result<PathBuf, String> {
    if content.len() > MAX_FILE_BYTES {
        return Err("файл больше 1 МБ".into());
    }
    let parts = safe_relative(rel)?;
    let base = files_dir(data_dir, workflow_id);
    std::fs::create_dir_all(&base).map_err(|e| format!("папка файлов: {e}"))?;
    let mut target = base.clone();
    parts.iter().for_each(|p| target.push(p));
    let parent = target.parent().ok_or("нет папки для файла")?.to_path_buf();
    std::fs::create_dir_all(&parent).map_err(|e| format!("папка файла: {e}"))?;
    let (base_real, parent_real) = (base.canonicalize().map_err(|e| e.to_string())?, parent.canonicalize().map_err(|e| e.to_string())?);
    if !parent_real.starts_with(&base_real) {
        return Err("путь выходит из папки процесса".into());
    }
    if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink() || m.is_dir()) {
        return Err("по этому пути нельзя записать файл".into());
    }
    std::fs::write(&target, content).map_err(|e| format!("запись файла: {e}"))?;
    Ok(target)
}

/// Что уже подключено: единственное место у каждого мессенджера. `place_id` обязан совпасть с ним.
fn linked_place(app: &App, target: &str) -> String {
    let settings = app.store.settings();
    match target {
        "telegram" => settings.telegram.chat_id,
        "discord" => settings.discord.channel_id,
        "slack" => settings.slack.channel_id,
        _ => String::new(),
    }
}

async fn post_chat(app: &Arc<App>, title: &str, bot_id: &str, text: &str) -> Result<(), String> {
    if app.store.bot(bot_id).is_none() {
        return Err("бота нет".into());
    }
    // Просто сообщение в тред: ход бота не начинается.
    let body = format!("Рабочий процесс «{title}» прислал:\n\n{text}");
    let entry = json!({ "role": "assistant", "text": body, "source": "workflow", "at": now() });
    let saved = app.store.append_message(bot_id, entry).await.map_err(|e| e.to_string())?;
    app.turns.emit(json!({ "type": "message", "botId": bot_id, "message": saved }));
    Ok(())
}

/// Выполнить вывод. Возвращает текст, который станет выходом шага.
pub async fn deliver(app: &Arc<App>, workflow_id: &str, title: &str, target: &str, node: &Node, input: &str) -> Result<String, String> {
    match target {
        "notify" => {
            let body: String = input.trim().chars().take(500).collect();
            let list = crate::reminders::send_text(app, &format!("Рабочий процесс «{title}»"), &body, &["windows".to_string()]).await;
            match list.into_iter().find_map(|d| d.error) {
                Some(e) => Err(format!("не удалось показать уведомление: {e}")),
                None => Ok(String::new()),
            }
        }
        "chat" => {
            post_chat(app, title, node.text("bot_id").trim(), input).await?;
            Ok(String::new())
        }
        "telegram" | "discord" | "slack" => {
            let wanted = node.text("place_id");
            let linked = linked_place(app, target);
            if !wanted.trim().is_empty() && wanted.trim() != linked {
                return Err("это место не подключено: пишу только туда, куда человек уже привязал мессенджер".into());
            }
            app.workflows.lock().note_sent(input);
            match target {
                "telegram" => crate::telegram::push_text(app, input).await,
                "discord" => crate::discord::push_text(app, input).await,
                _ => crate::slack::push_text(app, input).await,
            }?;
            Ok(String::new())
        }
        "file" => {
            let (data_dir, wf, rel, text) = (app.store.data_dir().to_path_buf(), workflow_id.to_string(), node.text("path"), input.to_string());
            let path = tokio::task::spawn_blocking(move || write_file(&data_dir, &wf, &rel, &text)).await.map_err(|e| e.to_string())??;
            Ok(path.to_string_lossy().into_owned())
        }
        other => Err(format!("неизвестный вывод «{other}»")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_workflow_folder() {
        for bad in ["", "/etc/passwd", "\\\\srv\\share\\a.txt", "C:\\x.txt", "c:x", "../a.txt", "a/../../b", "a\\..\\b", "con.txt", "dir/NUL", "COM1", "a/lpt3.log", "a.", "a /b", "x?.txt", "a\u{0}b", "a//b"] {
            assert!(safe_relative(bad).is_err(), "{bad:?} должен быть отклонён");
        }
        assert_eq!(safe_relative("отчёты/2026/итог.txt").unwrap(), vec!["отчёты", "2026", "итог.txt"]);
    }

    #[test]
    fn files_are_written_only_below_the_folder_and_limited() {
        let dir = std::env::temp_dir().join(format!("wf-io-{}", now()));
        let path = write_file(&dir, "wf_a", "out/report.txt", "привет").unwrap();
        assert!(path.starts_with(files_dir(&dir, "wf_a").canonicalize().unwrap_or_else(|_| files_dir(&dir, "wf_a"))) || path.starts_with(files_dir(&dir, "wf_a")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "привет");
        assert!(write_file(&dir, "wf_a", "../escape.txt", "x").is_err());
        assert!(write_file(&dir, "wf_a", "big.txt", &"я".repeat(MAX_FILE_BYTES)).is_err());
        assert!(write_file(&dir, "wf_a", "out", "x").is_err(), "папку файлом не заменить");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
