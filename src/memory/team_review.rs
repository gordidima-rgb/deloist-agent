//! «Разбор памяти команды»: оркестратор — доктор памяти своей команды (владелец, 17.11.2026).
//!
//! Раз в день тихая рутина оркестратора читает память команды и факты участников, сливает дубли, дополняет пустые
//! факты, предлагает человеку переносы и правила, а итог пишет в чат команды. Общий доктор памяти команды не видит.
//! Инструменты только для оркестратора и только про его команду: чужая команда для него не существует,
//! личные факты участников он читает, но не правит, факты «от человека» не меняются никогда.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::{bare_id, commit, dir, forget_prompts_for, load, normalize_entities, resolve_team, room_scope, who_bot, Book, Change, Fact, BODY_LIMIT, GATE, SHARED, TITLE_LIMIT};
use crate::sensitive::secret_in;
use crate::service;
use crate::store::{now, Bot, Room, Routine};
use crate::App;

/// Метка в начале постановки рутины: по ней находим рутину команды и не заводим вторую.
const MARK: &str = "[разбор-памяти-команды:";
/// Час запуска по часам человека. Фиксированное время с догоном 12 часов: ночью компьютер может быть выключен.
const AT: &str = "12:00";
const MAX_DROPS: usize = 8;
const REVIEW_LIMIT: usize = 14_000;
const REPORT_LIMIT: usize = 1_500;

pub fn is_tool(name: &str) -> bool {
    matches!(name, "team_memory_review" | "team_memory_merge" | "team_memory_report")
}

pub fn defs() -> Vec<Value> {
    let function = |name: &str, description: &str, properties: Value, required: &[&str]| {
        let mut parameters = json!({ "type": "object", "properties": properties });
        if !required.is_empty() {
            parameters["required"] = json!(required);
        }
        json!({ "type": "function", "function": { "name": name, "description": description, "parameters": parameters } })
    };
    let team = json!({ "type": "string", "description": "Название команды, если ты ведёшь несколько. Без него — твоя единственная." });
    vec![
        function(
            "team_memory_review",
            "Только для оркестратора. Показать всё, что нужно для разбора памяти своей команды: факты памяти команды, личные факты участников (только чтение), \
             факты участников в общей памяти (их memory.md — среди личных фактов). Чужие команды не показываются.",
            json!({ "team": team }),
            &[],
        ),
        function(
            "team_memory_merge",
            "Только для оркестратора. Слить дубли в памяти своей команды: факт keep_id остаётся (при желании с новым title/body), факты drop_ids удаляются, \
             связи переходят к оставленному. Запись идёт в журнал памяти, «Отменить» работает. Факты с автором «человек» удалять нельзя, \
             а оставить без правок можно. Личные и общие факты этим инструментом не трогаются.",
            json!({
                "keep_id": { "type": "string", "description": "Номер факта, который остаётся: m12." },
                "drop_ids": { "type": "array", "items": { "type": "string" }, "description": "Номера дублей, которые сливаются в keep_id." },
                "title": { "type": "string", "description": "Новый заголовок оставленного факта. Необязательно." },
                "body": { "type": "string", "description": "Новый текст оставленного факта: сведи в него всё важное из дублей. Необязательно." },
                "team": team,
            }),
            &["keep_id", "drop_ids"],
        ),
        function(
            "team_memory_report",
            "Только для оркестратора. Написать в чат своей команды короткий итог разбора памяти: что слито, что дополнено, что предлагаешь человеку. До 8 строк.",
            json!({ "text": { "type": "string", "description": "Итог разбора." }, "team": team }),
            &["text"],
        ),
    ]
}

pub async fn exec(app: &Arc<App>, bot: &Bot, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "team_memory_review" => review(app, bot, args).await,
        "team_memory_merge" => merge(app, bot, args).await,
        _ => report(app, bot, args).await,
    }
}

/// Команда оркестратора: только он и только его.
fn team_of(rooms: &[Room], bot: &Bot) -> Result<Room, String> {
    if !bot.orchestrator {
        return Err("этот инструмент только для оркестратора команды".into());
    }
    let scope = resolve_team(rooms, &bot.id)?;
    rooms.iter().find(|r| room_scope(&r.id) == scope).cloned().ok_or_else(|| "такой команды нет".into())
}

fn author_name(author: &str, bots: &[Bot]) -> String {
    match author {
        "human" => "человек".into(),
        "doctor" => "доктор памяти".into(),
        other => bots.iter().find(|b| b.id == other).map(|b| b.name.clone()).unwrap_or_else(|| other.to_string()),
    }
}

fn fact_block(fact: &Fact, bots: &[Bot]) -> String {
    let body = if fact.body.trim().is_empty() { "(пусто)".to_string() } else { fact.body.trim().to_string() };
    format!("[{}] {} · автор: {} — {}\n  {}", fact.id, super::kind_label(&fact.kind), author_name(&fact.author, bots), fact.title, body.replace('\n', "\n  "))
}

/// Всё для разбора одной команды. Другие команды и чужие личные факты сюда не попадают.
pub(crate) fn review_text(book: &Book, room: &Room, bots: &[Bot], orchestrator: &str) -> String {
    let scope = room_scope(&room.id);
    let mut out = format!("# Память команды «{}» ({})\n", room.name, scope);
    let team: Vec<&Fact> = book.facts.iter().filter(|f| f.scope == scope).collect();
    if team.is_empty() {
        out.push_str("пока пусто\n");
    }
    for fact in team {
        out.push_str(&fact_block(fact, bots));
        out.push('\n');
    }
    let members: Vec<&Bot> = room.bot_ids.iter().filter(|id| *id != orchestrator).filter_map(|id| bots.iter().find(|b| &b.id == id)).collect();
    for member in &members {
        out.push_str(&format!("\n# {} — {} (личные факты, только чтение)\n", member.name, member.role));
        let own: Vec<&Fact> = book.facts.iter().filter(|f| f.scope == member.id).collect();
        if own.is_empty() {
            out.push_str("личных фактов нет\n");
        }
        for fact in own {
            out.push_str(&fact_block(fact, bots));
            out.push('\n');
        }
    }
    let ids: HashSet<&str> = members.iter().map(|b| b.id.as_str()).collect();
    let shared: Vec<&Fact> = book.facts.iter().filter(|f| f.scope == SHARED && ids.contains(f.author.as_str())).collect();
    out.push_str("\n# Общая память: факты, записанные участниками команды\n");
    if shared.is_empty() {
        out.push_str("нет\n");
    }
    for fact in shared {
        out.push_str(&fact_block(fact, bots));
        out.push('\n');
    }
    service::take_chars(&out, REVIEW_LIMIT)
}

pub async fn review(app: &Arc<App>, bot: &Bot, _args: &Value) -> Result<String, String> {
    let room = team_of(&app.store.rooms(), bot)?;
    let book = load(app).await;
    let bots = app.store.bots();
    Ok(review_text(&book, &room, &bots, &bot.id))
}

/// Слияние дублей внутри памяти команды. Меняет книгу и возвращает изменения для журнала.
pub(crate) fn merge_in(book: &mut Book, scope: &str, keep: &str, drops: &[String], title: Option<&str>, body: Option<&str>, at: i64) -> Result<Vec<Change>, String> {
    if drops.is_empty() || drops.len() > MAX_DROPS {
        return Err(format!("drop_ids: от 1 до {MAX_DROPS} номеров"));
    }
    if drops.iter().any(|d| d == keep) {
        return Err("keep_id нельзя сливать самого в себя".into());
    }
    let find = |book: &Book, id: &str| -> Result<usize, String> {
        let index = book.facts.iter().position(|f| f.id == id).ok_or_else(|| format!("факта [{id}] нет"))?;
        // Чужая память для оркестратора не существует: не отличаем «нет» от «не твоё».
        if book.facts[index].scope != scope {
            return Err(format!("факта [{id}] нет в памяти этой команды"));
        }
        Ok(index)
    };
    let keep_index = find(book, keep)?;
    let mut drop_facts = Vec::new();
    for id in drops {
        let fact = book.facts[find(book, id)?].clone();
        if fact.author == "human" {
            return Err(format!("[{id}] записал человек: удалять его нельзя, оставь его главным (keep_id) без правок"));
        }
        drop_facts.push(fact);
    }
    let kept_by_human = book.facts[keep_index].author == "human";
    if kept_by_human && (title.is_some() || body.is_some()) {
        return Err(format!("[{keep}] записал человек: править его нельзя, слей в него дубли без title и body"));
    }
    let mut changes: Vec<Change> = Vec::new();
    let before = book.facts[keep_index].clone();
    if !kept_by_human {
        let mut entities = before.entities.clone();
        let mut links = before.links.clone();
        for fact in &drop_facts {
            entities.extend(fact.entities.clone());
            links.extend(fact.links.clone());
        }
        let fact = &mut book.facts[keep_index];
        if let Some(title) = title.map(super::one_line).filter(|t| !t.is_empty()) {
            fact.title = service::take_chars(&title, TITLE_LIMIT);
        }
        if let Some(body) = body {
            fact.body = service::take_chars(body.trim(), BODY_LIMIT);
        }
        fact.entities = normalize_entities(&entities);
        let mut seen = HashSet::new();
        fact.links = links.into_iter().filter(|l| l != keep && !drops.contains(l) && seen.insert(l.clone())).collect();
        fact.updated_at = at;
        changes.push((Some(before), Some(fact.clone())));
    }
    for fact in drop_facts {
        book.facts.retain(|f| f.id != fact.id);
        changes.push((Some(fact), None));
    }
    // Связи на слитые факты ведут к оставленному — только в памяти этой команды и не у фактов человека.
    for fact in book.facts.iter_mut().filter(|f| f.scope == scope && f.author != "human" && f.id != keep) {
        if fact.links.iter().any(|l| drops.contains(l)) {
            let before = fact.clone();
            let own = fact.id.clone();
            let mut seen = HashSet::new();
            fact.links = fact.links.iter().map(|l| if drops.contains(l) { keep.to_string() } else { l.clone() }).filter(|l| *l != own && seen.insert(l.clone())).collect();
            changes.push((Some(before), Some(fact.clone())));
        }
    }
    Ok(changes)
}

pub async fn merge(app: &Arc<App>, bot: &Bot, args: &Value) -> Result<String, String> {
    let room = team_of(&app.store.rooms(), bot)?;
    let scope = room_scope(&room.id);
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
    let keep = bare_id(&text("keep_id").unwrap_or_default());
    let drops: Vec<String> = args.get("drop_ids").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(bare_id).collect()).unwrap_or_default();
    let (title, body) = (text("title"), text("body"));
    if secret_in(&format!("{}\n{}", title.as_deref().unwrap_or_default(), body.as_deref().unwrap_or_default())).is_some() {
        return Err(super::SECRET_REFUSAL.into());
    }
    let entry = {
        let _gate = GATE.lock().await;
        let mut book = load(app).await;
        let changes = merge_in(&mut book, &scope, &keep, &drops, title.as_deref(), body.as_deref(), now())?;
        let note = format!("{}: слияние дублей — память команды «{}»", bot.name, room.name);
        commit(app, &book, who_bot(bot), "merge", &note, changes, json!({})).await.map_err(|e| e.to_string())?
    };
    forget_prompts_for(app, &[scope]).await;
    Ok(format!("Слито: [{}] остаётся, убрано {}.{}", keep, drops.len(), entry.map(|e| format!(" [журнал: {e}]")).unwrap_or_default()))
}

pub async fn report(app: &Arc<App>, bot: &Bot, args: &Value) -> Result<String, String> {
    let room = team_of(&app.store.rooms(), bot)?;
    let text = args.get("text").and_then(Value::as_str).unwrap_or_default().trim();
    if text.is_empty() {
        return Err("пустой итог".into());
    }
    if secret_in(text).is_some() {
        return Err(super::SECRET_REFUSAL.into());
    }
    let text = service::take_chars(text, REPORT_LIMIT);
    let entry = json!({ "role": "assistant", "botId": bot.id, "name": bot.name, "text": text, "at": now() });
    let saved = app.store.append_room_message(&room.id, entry).await.map_err(|e| e.to_string())?;
    app.turns.emit(json!({ "type": "room_message", "roomId": room.id, "message": saved }));
    Ok(format!("Итог записан в чат команды «{}».", room.name))
}

// ---------------------------------------------------------------------------
// Рутина оркестратора
// ---------------------------------------------------------------------------

fn prompt_for(room: &Room) -> String {
    format!(
        "{MARK}{id}]\n\
         Ежедневный тихий разбор памяти команды «{name}». Ты её оркестратор и доктор её памяти. Человека рядом нет: ничего не спрашивай.\n\
         1. Вызови team_memory_review: память команды, личные факты участников (только чтение), их факты в общей памяти.\n\
         2. Наведи порядок в памяти команды. Дубли слей через team_memory_merge. Пустой факт, ответ на который очевиден из личных фактов участника, \
         дополни через remember (id факта, scope team). Факты с автором «человек» не правь и не удаляй.\n\
         3. Договорённости, которые касаются только этой команды, но лежат в общей памяти или у участников, сам не переноси и не удаляй: предложи человеку в итоге \
         («перенести [m12] в память команды?»).\n\
         4. Если у участника одно и то же «всегда» или «никогда» повторяется в фактах, предложи правило: себе — propose_rule, участнику — строкой в итоге \
         («правило для Автора: …»; правила участника лежат в его brain.md и меняются только человеком). Не больше трёх предложений за раз.\n\
         5. Если что-то изменено или предложено, вызови team_memory_report: до 8 строк, что слито, что дополнено, что предлагаешь. Если делать нечего — ничего не пиши и ответь одним словом: тихо.\n\
         Нельзя: править личные факты участников, трогать другие команды, записывать пароли и ключи.",
        id = room.id,
        name = room.name
    )
}

/// Ввод для `Store::add_routine_capped`: тихая, каждый день, от имени оркестратора.
pub(crate) fn routine_input(room: &Room, orchestrator: &Bot) -> Value {
    json!({
        "botId": orchestrator.id, "title": "Разбор памяти команды", "prompt": prompt_for(room),
        "at": AT, "days": [], "enabled": true, "quiet": true,
    })
}

fn mark_of(room_id: &str) -> String {
    format!("{MARK}{room_id}]")
}

/// Команды с оркестратором, которым рутину ещё не заводили.
pub(crate) fn pending(rooms: &[Room], bots: &[Bot], routines: &[Routine], created: &HashSet<String>) -> Vec<(Room, Bot)> {
    rooms
        .iter()
        .filter(|r| !r.archived && !created.contains(&r.id))
        .filter_map(|r| {
            let orch = r.bot_ids.iter().find_map(|id| bots.iter().find(|b| &b.id == id && b.orchestrator))?;
            let mark = mark_of(&r.id);
            (!routines.iter().any(|x| x.prompt.contains(&mark))).then(|| (r.clone(), orch.clone()))
        })
        .collect()
}

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn created_path(app: &App) -> std::path::PathBuf {
    dir(app).join("team_review.json")
}

async fn load_created(app: &App) -> HashSet<String> {
    let raw = tokio::fs::read_to_string(created_path(app)).await.unwrap_or_default();
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default().into_iter().collect()
}

/// Тик планировщика: у каждой команды с оркестратором есть своя рутина разбора. Заводим один раз на команду:
/// если человек её удалил, второй раз не возвращаем.
pub async fn ensure_routines(app: &Arc<App>) {
    let rooms = app.store.rooms();
    let bots = app.store.bots();
    // Рутины до 0.4.3 велели читать MAIN.md, которого больше нет: их задание обновляется до нынешнего.
    for routine in app.store.routines().into_iter().filter(|r| r.prompt.contains(MARK) && r.prompt.contains("MAIN.md")) {
        if let Some(room) = rooms.iter().find(|room| routine.prompt.contains(&mark_of(&room.id))) {
            if let Err(e) = app.store.update_routine(&routine.id, &json!({ "prompt": prompt_for(room) }), false).await {
                eprintln!("[memory] рутина разбора памяти «{}» не обновлена: {e}", room.name);
            }
        }
    }
    let known = SEEN.lock().unwrap().clone().unwrap_or_default();
    if pending(&rooms, &bots, &app.store.routines(), &known).is_empty() {
        return;
    }
    let mut created = load_created(app).await;
    created.extend(known);
    let todo = pending(&rooms, &bots, &app.store.routines(), &created);
    for (room, orch) in todo {
        match app.store.add_routine_capped(&routine_input(&room, &orch), crate::routine_tools::MAX_ROUTINES, false).await {
            Ok(Some(_)) => {
                created.insert(room.id.clone());
            }
            Ok(None) => break,
            Err(e) => eprintln!("[memory] рутина разбора памяти команды «{}»: {e}", room.name),
        }
    }
    // Рутина, найденная по метке, тоже считается заведённой: человек мог её переименовать или выключить.
    let routines = app.store.routines();
    for room in rooms.iter().filter(|r| !r.archived) {
        if routines.iter().any(|x| x.prompt.contains(&mark_of(&room.id))) {
            created.insert(room.id.clone());
        }
    }
    let list: Vec<&String> = created.iter().collect();
    let _ = tokio::fs::create_dir_all(dir(app)).await;
    let _ = tokio::fs::write(created_path(app), serde_json::to_string(&list).unwrap_or_default()).await;
    *SEEN.lock().unwrap() = Some(created);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(id: &str, scope: &str, author: &str, title: &str, body: &str) -> Fact {
        Fact { id: id.into(), scope: scope.into(), kind: "project".into(), title: title.into(), body: body.into(), author: author.into(), ..Fact::default() }
    }

    fn bot(id: &str, name: &str, orchestrator: bool) -> Bot {
        Bot { id: id.into(), name: name.into(), orchestrator, ..Bot::default() }
    }

    fn room(id: &str, bots: &[&str]) -> Room {
        Room { id: id.into(), name: format!("Команда {id}"), bot_ids: bots.iter().map(|b| b.to_string()).collect(), ..Room::default() }
    }

    fn book() -> Book {
        Book {
            next: 20,
            facts: vec![
                fact("m1", "room:r1", "b2", "Посты — по средам", "по средам"),
                fact("m2", "room:r1", "b2", "Посты выходят в среду", ""),
                fact("m3", "room:r1", "human", "Тон — спокойный", "от человека"),
                fact("m4", "room:r2", "b9", "Чужая команда: секрет плана", "x"),
                fact("m5", "b2", "b2", "Личное Автора", "личное"),
                fact("m6", "b9", "b9", "Личное чужого", "x"),
                fact("m7", SHARED, "b2", "Общий факт Автора", "y"),
                fact("m8", SHARED, "b9", "Общий факт чужого", "z"),
            ],
            ..Book::default()
        }
    }

    #[test]
    fn review_shows_own_team_and_hides_other_teams() {
        let bots = vec![bot("b1", "Оркестратор", true), bot("b2", "Автор", false), bot("b9", "Чужой", false)];
        let text = review_text(&book(), &room("r1", &["b1", "b2"]), &bots, "b1");
        for need in ["Посты — по средам", "Тон — спокойный", "Личное Автора", "Общий факт Автора", "автор: человек"] {
            assert!(text.contains(need), "нет «{need}»: {text}");
        }
        for hidden in ["Чужая команда", "Личное чужого", "Общий факт чужого"] {
            assert!(!text.contains(hidden), "виден «{hidden}»: {text}");
        }
    }

    #[test]
    fn tools_are_for_an_orchestrator_and_only_for_its_own_team() {
        let rooms = vec![room("r1", &["b1", "b2"]), room("r2", &["b9"])];
        assert!(team_of(&rooms, &bot("b2", "Автор", false)).is_err(), "обычный участник не разбирает память");
        assert_eq!(team_of(&rooms, &bot("b1", "О", true)).unwrap().id, "r1");
    }

    #[test]
    fn merge_joins_duplicates_inside_the_team() {
        let mut b = book();
        b.facts[0].entities = vec!["Reddit".into()];
        b.facts[1].entities = vec!["Среда".into()];
        let changes = merge_in(&mut b, "room:r1", "m1", &["m2".to_string()], Some("Посты выходят по средам"), Some("по средам, 10:00"), 5).unwrap();
        assert_eq!(changes.len(), 2);
        assert!(b.facts.iter().all(|f| f.id != "m2"));
        let kept = b.facts.iter().find(|f| f.id == "m1").unwrap();
        assert_eq!(kept.body, "по средам, 10:00");
        assert_eq!(kept.entities.len(), 2, "{:?}", kept.entities);
    }

    #[test]
    fn merge_never_touches_human_written_facts_or_other_memories() {
        let mut b = book();
        let before = b.facts.clone();
        for (keep, drop) in [("m1", "m3"), ("m1", "m4"), ("m1", "m5"), ("m1", "m7"), ("m4", "m1")] {
            assert!(merge_in(&mut b, "room:r1", keep, &[drop.to_string()], None, None, 5).is_err(), "{keep} <- {drop}");
        }
        // Оставить факт человека главным можно, но править его нельзя.
        assert!(merge_in(&mut b, "room:r1", "m3", &["m1".to_string()], Some("Другое"), None, 5).is_err());
        assert_eq!(b.facts, before, "после отказов память не изменилась");
        let changes = merge_in(&mut b, "room:r1", "m3", &["m1".to_string()], None, None, 5).unwrap();
        assert_eq!(changes.len(), 1, "факт человека не менялся, дубль ушёл");
        assert_eq!(b.facts.iter().find(|f| f.id == "m3"), before.iter().find(|f| f.id == "m3"));
    }

    #[test]
    fn a_routine_is_pending_for_each_live_team_with_an_orchestrator_once() {
        let bots = vec![bot("b1", "О", true), bot("b2", "А", false), bot("b3", "О2", true)];
        let mut archived = room("r3", &["b3"]);
        archived.archived = true;
        let rooms = vec![room("r1", &["b1", "b2"]), room("r2", &["b2"]), archived];
        let todo = pending(&rooms, &bots, &[], &HashSet::new());
        assert_eq!(todo.len(), 1, "только r1: без оркестратора и в архиве рутины нет");
        let (r, o) = &todo[0];
        let input = routine_input(r, o);
        assert_eq!(input["botId"], "b1");
        assert_eq!(input["quiet"], true);
        assert_eq!(input["at"], AT);
        // Рутина уже есть (или помечена созданной) — вторая не нужна, даже если человек её удалил.
        let existing = Routine { prompt: input["prompt"].as_str().unwrap().into(), ..Routine::default() };
        assert!(pending(&rooms, &bots, &[existing], &HashSet::new()).is_empty());
        assert!(pending(&rooms, &bots, &[], &HashSet::from(["r1".to_string()])).is_empty());
    }
}
