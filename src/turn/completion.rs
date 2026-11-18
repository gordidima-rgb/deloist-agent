pub(super) const MAX_CONTINUATIONS: usize = 4;
pub(super) const NUDGE: &str = "Заверши запуск вызовом finish_turn с итогом.";
pub(super) const PROMPT: &str = "Продолжи незавершённую часть исходного задания человека. Промежуточный отчёт уже показан. Выполни оставшиеся пункты полностью, сохраняя исходные ограничения и разрешения. Не повторяй готовую работу. Если появился реальный блокер или нужны новые данные человека, точно назови их; иначе продолжай работу до результата.";

/// `settled` — человеку ничего не ждёт: нет пропущенных карточек и открытых вопросов бота.
/// Ожидание человека важнее тишины: тихий запуск, пока вопрос висит, — «нужен ты», а не галочка.
pub(super) fn status_from_finish(finish: Option<&str>, quiet: bool, settled: bool) -> &'static str {
    match finish {
        Some("need_you") => "needs",
        Some("failed") => "failed",
        _ if !settled => "needs",
        _ if quiet => "quiet",
        _ => "ok",
    }
}

pub(super) fn should_nudge(calls_empty: bool, wants_finish: bool, nudged: bool, finish_status: Option<&str>) -> bool {
    calls_empty && wants_finish && !nudged && finish_status.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(status_from_finish(Some("nothing"), true, true), "quiet");
        assert_eq!(status_from_finish(Some("nothing"), false, true), "ok");
        assert_eq!(status_from_finish(Some("need_you"), false, true), "needs");
        assert_eq!(status_from_finish(Some("failed"), false, true), "failed");
        assert_eq!(status_from_finish(None, true, true), "quiet");
        assert_eq!(status_from_finish(None, true, false), "needs");
        assert_eq!(status_from_finish(Some("nothing"), true, false), "needs");
        assert_eq!(status_from_finish(None, false, false), "needs");
        assert_eq!(status_from_finish(None, false, true), "ok");
    }

    #[test]
    fn nudge_once() {
        assert!(should_nudge(true, true, false, None));
        assert!(!should_nudge(false, true, false, None));
        assert!(!should_nudge(true, false, false, None));
        assert!(!should_nudge(true, true, true, None));
        assert!(!should_nudge(true, true, false, Some("done")));
    }
}
