use serde_json::Value;

pub(super) const MAX_CONTINUATIONS: usize = 4;
pub(super) const PROMPT: &str = "Продолжи незавершённую часть исходного задания человека. Промежуточный отчёт уже показан. Выполни оставшиеся пункты полностью, сохраняя исходные ограничения и разрешения. Не повторяй готовую работу. Если появился реальный блокер или нужны новые данные человека, точно назови их; иначе продолжай работу до результата.";

/// An ACP end_turn is not evidence that the user's task is done. Only retry an explicit
/// unfinished progress report after successful work, within the existing turn budget.
pub(super) fn should_continue(driver: &str, text: &str, activity: &[Value], request: &str, count: usize) -> bool {
    if !(driver == "acp" || driver.starts_with("acp:")) || count >= MAX_CONTINUATIONS {
        return false;
    }
    let text = text.to_lowercase();
    let request = request.to_lowercase();
    if ["не продолжай", "остановись", "только промежуточный", "только отчёт", "только отчет", "частичный отчёт", "частичный отчет", "промежуточный отчёт", "промежуточный отчет", "progress only", "report only", "partial report", "stop after"]
        .iter().any(|phrase| request.contains(phrase)) {
        return false;
    }
    // Ignore quoted material and code blocks: it may be source content rather than the agent's report.
    let mut in_code = false;
    let report = text.lines().filter_map(|line| {
        let line = line.trim();
        if line.starts_with("```") { in_code = !in_code; return None; }
        if in_code || line.starts_with('>') || line.starts_with('"') || line.starts_with('«') { None } else { Some(line) }
    }).collect::<Vec<_>>().join("\n");
    let mut unquoted = String::new();
    let mut quote = None;
    for c in report.chars() {
        match (quote, c) {
            (None, '"' | '«' | '`') => quote = Some(c),
            (Some('«'), '»') | (Some('"'), '"') | (Some('`'), '`') => quote = None,
            (None, _) => unquoted.push(c),
            _ => {}
        }
    }
    let incomplete = ["ещё не заверш", "еще не заверш", "не завершён", "не завершен", "ещё не закончен", "еще не закончен", "анализ не закончен", "работа не закончена", "not finished", "not yet complete", "not completed", "still unfinished"]
        .iter().any(|phrase| unquoted.contains(phrase));
    if !incomplete {
        return false;
    }
    if report.trim_end().ends_with('?') {
        return false;
    }
    if ["нужен вход", "необходим вход", "требуется вход", "нужно войти", "нужно авторизоваться", "нужна твоя помощь", "жду тво", "нет доступа", "доступа нет", "не могу продолжить", "не могу получить", "продолжить не удалось", "не удалось продолжить", "требуется подтверждение", "нужны данные", "требуются данные", "недостаточно данных", "капч", "captcha", "rate limit", "quota", "нужен ответ", "нужно уточнить", "need your", "cannot continue", "can't continue", "blocked", "sign in", "log in"]
        .iter().any(|phrase| text.contains(phrase)) {
        return false;
    }
    if activity.iter().any(|call| matches!(call["status"].as_str(), Some("denied" | "error"))) {
        return false;
    }
    activity.iter().any(|call| call["status"] == "ok" && call["kind"].as_str().is_some_and(|kind| kind != "think" && kind != "permission"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unfinished_account_analysis_continues_after_real_progress() {
        let work = [json!({"kind":"execute","status":"ok","name":"mcp.deloist_browser.browser_page"})];
        assert!(should_continue("acp", "Доступ к X работает. Добавил AppuBot и Turnstone. **Анализ ещё не завершён:** остальные аккаунты нужно изучить.", &work, "открыл доступ", 0));
        assert!(!should_continue("acp", "Анализ всех аккаунтов завершён. Результаты сохранены.", &work, "все аккаунты", 0));
    }

    #[test]
    fn access_problems_and_denials_never_get_automatic_retries() {
        let work = [json!({"kind":"execute","status":"ok"})];
        for blocker in ["Нужен вход в X.", "Нет доступа к остальным аккаунтам.", "Жду твоего решения.", "Нужно уточнить список.", "Blocked: need your credentials."] {
            assert!(!should_continue("acp", &format!("Анализ ещё не завершён. {blocker}"), &work, "все аккаунты", 0), "{blocker}");
        }
        assert!(!should_continue("acp", "Анализ ещё не завершён.", &[json!({"status":"denied"})], "все аккаунты", 0));
    }

    #[test]
    fn explicit_partial_requests_and_the_continuation_budget_are_respected() {
        let work = [json!({"kind":"execute","status":"ok"})];
        assert!(!should_continue("acp", "Анализ ещё не завершён.", &work, "Дай только промежуточный отчёт", 0));
        assert!(!should_continue("acp", "Анализ ещё не завершён.", &work, "все аккаунты", MAX_CONTINUATIONS));
        assert!(!should_continue("acp", "Анализ ещё не завершён.", &[], "все аккаунты", 0));
        assert!(!should_continue("api", "Анализ ещё не завершён.", &work, "все аккаунты", 0));
        assert!(should_continue("acp:codex-acp", "Анализ ещё не завершён.", &work, "все аккаунты", 0));
        assert!(!should_continue("acp", "В статье сказано: «анализ ещё не завершён». Готово.", &work, "все аккаунты", 0));
        assert!(!should_continue("acp", "Анализ ещё не завершён.", &[json!({"kind":"execute", "status":"running"})], "все аккаунты", 0));
    }
}
