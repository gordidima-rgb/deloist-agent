//! Бот собирает команду инструментом `propose_team`, а не скриптом.
//!
//! Инструмент ничего не создаёт: он кладёт в `offers` предложение, показывает человеку карточку
//! «Создать / Не надо» и ждёт решения. По «Создать» программа создаёт ровно то, что лежит в
//! сохранённом предложении, а не то, что агент прислал бы повторно. Принять карточку может только
//! окно (`guard::human_only`: `/api/bots/{id}/offers/…`). Новые боты получают безопасные умолчания:
//! режим вопросов `ask`, без сайтов, без предела токенов, драйвер и модель предложившего бота.
//! Поля прав в аргументах (`approval`, `browserSites`, `driver`, команды, папки) не читаются вообще.
//!
//! Тут же `search_catalog`: бот ищет в каталоге готовых агентов, чтобы предложить их карточкой.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::offers::Offer;
use crate::store::{uid, Bot};
use crate::App;

/// Сколько ботов можно предложить за раз: новых и из каталога вместе.
pub const MAX_NEW_BOTS: usize = 5;
pub const MAX_INSTRUCTIONS: usize = 4_000;
pub const MAX_NAME: usize = 40;
pub const MAX_ROLE: usize = 80;
/// Пометка в строке доступа: папка команды — это папка запущенной программы.
const PROGRAM_FOLDER_MARK: &str = "ПАПКА С ПРОГРАММОЙ";
/// Сколько инструмент ждёт человека.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SEARCH_LIMIT: usize = 10;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBot {
    pub name: String,
    pub role: String,
    pub instructions: String,
}

/// Бот из каталога: ставится существующей установкой каталога (`catalog::install`).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPick {
    pub id: String,
    pub name: String,
    pub role: String,
    pub needs: Vec<String>,
    pub installed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingBot {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomPlan {
    /// Пусто — команда новая.
    pub id: String,
    pub name: String,
    pub is_new: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamPlan {
    pub new_bots: Vec<NewBot>,
    pub catalog_bots: Vec<CatalogPick>,
    pub add_bots: Vec<ExistingBot>,
    pub room: Option<RoomPlan>,
    /// Имя ведущего (как в карточке). Пусто — по умолчанию.
    pub lead: String,
    /// Что получат новые участники от общих папок и навыков команды: строки для карточки.
    pub access: Vec<String>,
    /// Отпечаток папок и навыков команды в момент предложения: если они сменились, карточка устарела.
    pub access_key: String,
    pub host_name: String,
    pub host_driver: String,
    pub host_model: String,
    /// Карточку предложил оркестратор: состав идёт в его команду, а она переходит в `ready`.
    pub host_orchestrator: bool,
    pub host_extra: Vec<(String, String)>,
}

impl TeamPlan {
    /// Так предложение видит окно: состав, доступ и с какими настройками родятся новые боты.
    pub fn event_json(&self, offer: &Offer) -> Value {
        json!({
            "id": offer.id,
            "hostBotId": offer.host_bot_id,
            "newBots": self.new_bots,
            "catalogBots": self.catalog_bots,
            "addBots": self.add_bots,
            "room": self.room,
            "lead": self.lead,
            "access": self.access,
            // Строки про доступ к папке с программой окно показывает предупреждением.
            "accessWarn": self.access.iter().map(|line| line.contains(PROGRAM_FOLDER_MARK)).collect::<Vec<_>>(),
            "defaults": { "approval": "ask", "hostName": self.host_name, "driver": self.host_driver, "model": self.host_model },
            "at": offer.at,
        })
    }
}

pub(crate) fn clean_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strings(args: &Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).ok_or_else(|| format!("{key}: нужен список непустых строк")))
            .collect(),
        Some(_) => Err(format!("{key}: нужен список")),
    }
}

/// Невидимые знаки форматирования (категория Cf: переворот строки, нулевая ширина, мягкий перенос и т. п.):
/// в имени они подделывают то, что видит человек в карточке.
pub(crate) fn is_format_char(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0xE0001 | 0xE0020..=0xE007F)
}

/// Невидимый знак в инструкциях: любой знак форматирования (Cf), кроме соединителей эмодзи (ZWJ, ZWNJ), и смена направления.
pub(crate) fn hidden_in_instructions(c: char) -> bool {
    (is_format_char(c) && !matches!(c, '‌' | '‍')) || is_bidi_control(c)
}

/// Знаки смены направления текста: в инструкциях запрещены (остальные Cf, например соединитель эмодзи, допустимы).
pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(c as u32, 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

pub(crate) fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Что показывать про доступ команды: папки, программные папки и навыки.
fn room_access(app: &App, room: &crate::store::Room) -> (Vec<String>, String) {
    let skills: Vec<String> = room.skills.iter().map(|id| app.store.skill(id).map(|s| s.name).unwrap_or_else(|| id.clone())).collect();
    let mut parts = Vec::new();
    if !room.folders.is_empty() {
        let list: Vec<String> = room
            .folders
            .iter()
            .map(|f| if room.program_folders.iter().any(|p| p.eq_ignore_ascii_case(f)) { format!("{f} ({PROGRAM_FOLDER_MARK}: из неё можно запускать код)") } else { f.clone() })
            .collect();
        parts.push(format!("папки: {}", list.join(", ")));
    }
    if !skills.is_empty() {
        parts.push(format!("навыки: {}", skills.join(", ")));
    }
    let key = format!("{:?}|{:?}|{:?}", room.folders, room.program_folders, room.skills);
    (parts, key)
}

fn catalog_needs(item: &crate::catalog::Item) -> Vec<String> {
    let agent = &item.manifest["agent"];
    let mut needs = Vec::new();
    let skills: Vec<String> = agent["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| crate::catalog::catalog().get(id).map(|s| crate::catalog::loc(&s.manifest["title"], "ru")).unwrap_or_else(|| id.to_string()))
        .collect();
    if !skills.is_empty() {
        needs.push(format!("навыки: {}", skills.join(", ")));
    }
    let sites: Vec<&str> = agent["sites"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !sites.is_empty() {
        needs.push(format!("работает с сайтами: {} (доступ не выдаётся, пока не разрешишь сам)", sites.join(", ")));
    }
    needs
}

fn new_room(app: &App, name: &str) -> Result<RoomPlan, String> {
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err(format!("room: имя новой команды до {MAX_NAME} знаков, без управляющих и невидимых символов"));
    }
    if app.store.rooms().iter().any(|r| same(&r.name, name)) {
        return Err(format!("команда «{name}» уже есть: укажи её именем в room, а не как новую"));
    }
    Ok(RoomPlan { id: String::new(), name: name.to_string(), is_new: true })
}

/// Разбор аргументов в план. Ошибка приходит агенту до карточки.
pub async fn plan_from(app: &Arc<App>, bot: &Bot, args: &Value) -> Result<TeamPlan, String> {
    let bots = app.store.bots();
    let taken = |name: &str| bots.iter().any(|b| same(&b.name, name));

    let mut new_bots: Vec<NewBot> = Vec::new();
    match args.get("newBots") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            for item in items {
                let name = clean_line(item.get("name").and_then(Value::as_str).unwrap_or_default());
                let role = clean_line(item.get("role").and_then(Value::as_str).unwrap_or_default());
                let instructions = item.get("instructions").and_then(Value::as_str).unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err("newBots: у каждого бота нужно имя".into());
                }
                if name.chars().count() > MAX_NAME || name.chars().any(|c| c.is_control() || is_format_char(c)) {
                    let shown: String = name.chars().filter(|c| !is_format_char(*c)).collect();
                    return Err(format!("newBots: имя «{shown}» длиннее {MAX_NAME} знаков или с управляющими и невидимыми символами"));
                }
                if role.chars().count() > MAX_ROLE {
                    return Err(format!("newBots: роль «{name}» длиннее {MAX_ROLE} знаков"));
                }
                if role.chars().any(|c| c.is_control() || is_format_char(c)) || instructions.chars().any(hidden_in_instructions) {
                    return Err(format!("newBots: роль или инструкции «{name}» с невидимыми символами (нулевой ширины, смены направления текста и т. п.)"));
                }
                if instructions.chars().count() > MAX_INSTRUCTIONS {
                    return Err(format!("newBots: инструкции «{name}» длиннее {MAX_INSTRUCTIONS} знаков: сократи"));
                }
                if taken(&name) {
                    return Err(format!("бот «{name}» уже есть: возьми его в addBots или выбери другое имя"));
                }
                if new_bots.iter().any(|b| same(&b.name, &name)) {
                    return Err(format!("newBots: имя «{name}» повторяется"));
                }
                new_bots.push(NewBot { name, role, instructions });
            }
        }
        Some(_) => return Err("newBots: нужен список объектов {name, role, instructions}".into()),
    }

    let mut catalog_bots: Vec<CatalogPick> = Vec::new();
    for id in strings(args, "catalogBots")? {
        if catalog_bots.iter().any(|c| c.id == id) {
            continue;
        }
        let item = crate::catalog::catalog().get(&id).ok_or_else(|| format!("в каталоге нет «{id}»: найди id через search_catalog"))?;
        if item.kind != "agent" {
            return Err(format!("«{id}» — не бот каталога (kind {}): в catalogBots только агенты", item.kind));
        }
        let name = clean_line(&crate::catalog::loc(&item.manifest["title"], "ru"));
        let state = crate::catalog::state_of(app, item).await;
        let installed = state["installed"].as_bool().unwrap_or(false);
        if !installed && (taken(&name) || new_bots.iter().any(|b| same(&b.name, &name))) {
            return Err(format!("бот «{name}» уже есть: имя из каталога занято"));
        }
        catalog_bots.push(CatalogPick {
            id,
            role: clean_line(&crate::catalog::loc(&item.manifest["agent"]["role"], "ru")).chars().take(MAX_ROLE * 2).collect(),
            needs: catalog_needs(item),
            installed,
            name,
        });
    }

    if new_bots.len() + catalog_bots.len() > MAX_NEW_BOTS {
        return Err(format!("за раз можно предложить до {MAX_NEW_BOTS} новых ботов (newBots и catalogBots вместе)"));
    }

    let mut add_bots: Vec<ExistingBot> = Vec::new();
    for name in strings(args, "addBots")? {
        let found = bots.iter().find(|b| same(&b.name, &name)).ok_or_else(|| format!("addBots: нет бота «{name}»"))?;
        if !add_bots.iter().any(|b| b.id == found.id) {
            add_bots.push(ExistingBot { id: found.id.clone(), name: found.name.clone() });
        }
    }
    if new_bots.is_empty() && catalog_bots.is_empty() && add_bots.is_empty() {
        return Err("нечего предлагать: нужны newBots, catalogBots или addBots".into());
    }

    // Оркестратор собирает состав в своей команде, а не в новой: что бы ни написал агент в room.
    let own_room = if bot.orchestrator { app.store.orchestrator_room(&bot.id) } else { None };
    let room = match args.get("room") {
        _ if own_room.is_some() => own_room.map(|r| RoomPlan { id: r.id, name: r.name, is_new: false }),
        None | Some(Value::Null) => None,
        Some(Value::String(name)) if name.trim().is_empty() => None,
        Some(Value::String(name)) if name.trim().to_lowercase().starts_with("new:") => {
            let fresh = clean_line(&name.trim()[4..]);
            Some(new_room(app, &fresh)?)
        }
        Some(Value::String(name)) => {
            let found = app
                .store
                .rooms()
                .into_iter()
                .find(|r| same(&r.name, name) && !r.archived)
                .ok_or_else(|| format!("room: нет команды «{}». Новую задай как {{\"new\": \"имя\"}}", name.trim()))?;
            Some(RoomPlan { id: found.id, name: found.name, is_new: false })
        }
        Some(Value::Object(map)) => {
            let name = clean_line(map.get("new").and_then(Value::as_str).unwrap_or_default());
            Some(new_room(app, &name)?)
        }
        Some(_) => return Err("room: имя команды или {\"new\": \"имя\"}".into()),
    };

    // Доступ получают только те, кто в команде ещё не состоит: общие папки и навыки есть у существующей команды.
    let (mut access, mut access_key) = (Vec::new(), String::new());
    let existing_room = room.as_ref().filter(|r| !r.is_new).and_then(|r| app.store.room(&r.id));
    if let Some(found) = &existing_room {
        let (parts, key) = room_access(app, found);
        access_key = key;
        let newcomers: Vec<String> = new_bots
            .iter()
            .map(|b| b.name.clone())
            .chain(catalog_bots.iter().map(|c| c.name.clone()))
            .chain(add_bots.iter().filter(|b| !found.bot_ids.contains(&b.id)).map(|b| b.name.clone()))
            .collect();
        if !newcomers.is_empty() && !parts.is_empty() {
            access.push(format!("Получат доступ к общему ({}): {}", newcomers.join(", "), parts.join("; ")));
        }
    }

    let lead = clean_line(args.get("lead").and_then(Value::as_str).unwrap_or_default());
    if !lead.is_empty() {
        if room.is_none() {
            return Err("lead указывают вместе с room".into());
        }
        let known = new_bots.iter().map(|b| b.name.as_str()).chain(catalog_bots.iter().map(|c| c.name.as_str())).chain(add_bots.iter().map(|b| b.name.as_str()));
        let in_room = existing_room.iter().flat_map(|r| r.bot_ids.iter()).filter_map(|id| bots.iter().find(|b| &b.id == id)).map(|b| b.name.as_str());
        if !known.chain(in_room).any(|n| same(n, &lead)) {
            return Err(format!("lead: «{lead}» нет среди участников команды"));
        }
    }

    if !lead.is_empty() {
        if let Some(found) = &existing_room {
            let current = bots.iter().find(|b| b.id == found.lead).map_or_else(|| "никто".to_string(), |b| b.name.clone());
            if !same(&current, &lead) {
                access.push(format!("Команда «{}» сменит ведущего: {current} → {lead}", found.name));
            }
        }
    }

    let host_extra: Vec<(String, String)> = ["modelLevel", "effort"]
        .iter()
        .filter_map(|k| bot.extra.get(*k).and_then(Value::as_str).map(|v| (k.to_string(), v.to_string())))
        .collect();
    Ok(TeamPlan { new_bots, catalog_bots, add_bots, room, lead, access, access_key, host_name: bot.name.clone(), host_driver: bot.driver.clone(), host_model: bot.model.clone(), host_extra, host_orchestrator: bot.orchestrator })
}

/// Инструмент `propose_team`: карточка человеку и ожидание решения.
pub async fn propose(app: &Arc<App>, bot: &Bot, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
    let plan = plan_from(app, bot, args).await?;
    // Тот же состав уже ждёт ответа: второй карточки не показываем, ждём то же решение.
    let (offer_id, rx) = match app.offers.find_team(&bot.id, &plan) {
        Some(id) => {
            let rx = app.offers.open_decision(&id);
            (id, rx)
        }
        None => {
            let offer = Offer {
                id: uid("offer"),
                host_bot_id: bot.id.clone(),
                specialist_id: String::new(),
                reason: String::new(),
                task: String::new(),
                at: crate::store::now(),
                team: Some(plan.clone()),
                change: None,
            };
            let rx = app.offers.open_decision(&offer.id);
            app.offers.add(offer.clone());
            app.turns.emit(json!({ "type": "team_proposal", "botId": bot.id, "proposal": plan.event_json(&offer) }));
            (offer.id, rx)
        }
    };
    await_decision(app, &bot.id, &offer_id, rx, cancel, "человек не ответил на карточку команды: ничего не создано").await
}

/// Ждать решения человека по карточке: итог, отмена хода или таймаут (тогда карточка снимается).
pub(crate) async fn await_decision(
    app: &Arc<App>,
    bot_id: &str,
    offer_id: &str,
    mut rx: tokio::sync::watch::Receiver<Option<String>>,
    cancel: &CancellationToken,
    timeout_message: &str,
) -> Result<String, String> {
    let waited = tokio::select! {
        done = rx.wait_for(|v| v.is_some()) => done.ok().and_then(|v| v.clone()),
        _ = cancel.cancelled() => None,
        _ = tokio::time::sleep(DECISION_TIMEOUT) => None,
    };
    match waited {
        Some(outcome) => Ok(outcome),
        None => {
            // Ход остановлен или человек не ответил: карточка больше не действует.
            app.offers.take(bot_id, offer_id);
            app.offers.decide(offer_id, "Предложение снято: человек не ответил вовремя.".into());
            Err(if cancel.is_cancelled() { "ход остановлен, карточка снята".into() } else { timeout_message.into() })
        }
    }
}

/// «Не надо»: ждущий инструмент получает отказ.
pub fn decline(app: &App, offer: &Offer) {
    if offer.team.is_some() || offer.change.is_some() {
        app.offers.decide(&offer.id, "Человек отказался: ничего не создано. Не предлагай то же снова без новой причины.".into());
    }
}

/// «Создать»: ровно то, что сохранено в предложении. Итог получает и ждущий инструмент.
pub async fn accept(app: &Arc<App>, offer: &Offer, plan: &TeamPlan) -> Result<String, String> {
    let result = build(app, plan).await;
    match &result {
        Ok(text) => app.offers.decide(&offer.id, text.clone()),
        Err(message) => app.offers.decide(&offer.id, format!("Не получилось создать: {message}")),
    }
    app.turns.emit(json!({ "type": "state_changed" }));
    result
}

async fn build(app: &Arc<App>, plan: &TeamPlan) -> Result<String, String> {
    // Мир мог поменяться, пока карточка висела: проверяем ещё раз и ничего не создаём вслепую.
    let bots = app.store.bots();
    for b in &plan.new_bots {
        if bots.iter().any(|x| same(&x.name, &b.name)) {
            return Err(format!("бот «{}» уже появился: предложи состав заново", b.name));
        }
    }
    for c in &plan.catalog_bots {
        let item = crate::catalog::catalog().get(&c.id).ok_or_else(|| format!("в каталоге больше нет «{}»", c.id))?;
        let installed = crate::catalog::state_of(app, item).await["installed"].as_bool().unwrap_or(false);
        let name = clean_line(&crate::catalog::loc(&item.manifest["title"], "ru"));
        if !installed && (bots.iter().any(|x| same(&x.name, &name)) || plan.new_bots.iter().any(|b| same(&b.name, &name))) {
            return Err(format!("бот «{name}» уже появился: имя из каталога занято, предложи состав заново"));
        }
    }
    for b in &plan.add_bots {
        if app.store.bot(&b.id).is_none() {
            return Err(format!("бота «{}» больше нет", b.name));
        }
    }
    let existing_room = match plan.room.as_ref().filter(|r| !r.is_new) {
        Some(r) => {
            let found = app.store.room(&r.id).ok_or_else(|| format!("команды «{}» больше нет", r.name))?;
            if room_access(app, &found).1 != plan.access_key {
                return Err(format!("папки или навыки команды «{}» изменились, пока карточка висела: предложи заново", r.name));
            }
            Some(found)
        }
        None => None,
    };

    let mut made: Vec<(String, String)> = Vec::new();
    let fail = |made: &[(String, String)], message: String| {
        let done: Vec<String> = made.iter().map(|(n, i)| format!("{n} ({i})")).collect();
        if done.is_empty() {
            message
        } else {
            format!("{message}. Уже создано: {}", done.join(", "))
        }
    };

    for b in &plan.new_bots {
        // Права не из агента: режим вопросов, пустые сайты и предел — умолчания, драйвер и модель — как у предложившего.
        let mut input = json!({
            "name": b.name,
            "role": b.role,
            "persona": b.instructions,
            "approval": "ask",
            "driver": plan.host_driver,
            "model": plan.host_model,
        });
        for (k, v) in &plan.host_extra {
            input[k] = json!(v);
        }
        let bot = app.store.add_bot(&input).await.map_err(|e| fail(&made, e.to_string()))?;
        made.push((bot.name, bot.id));
    }
    for c in &plan.catalog_bots {
        // Режим вопросов задаём самой установке, а не правим потом: бот не записывается с режимом из манифеста.
        // Сайты установка без явного согласия (`sites: true`) не даёт, а мы его не передаём.
        let body = json!({ "driver": plan.host_driver, "model": plan.host_model, "forceApproval": "ask" });
        let reply = crate::catalog::install(app, &c.id, &body).await.map_err(|e| fail(&made, e.message))?;
        let id = reply["bot"]["id"].as_str().unwrap_or_default().to_string();
        if id.is_empty() {
            return Err(fail(&made, format!("каталог не вернул бота для «{}»", c.id)));
        }
        if reply["already"].as_bool() != Some(true) {
            let safe = app.store.bot(&id).is_some_and(|b| b.approval == "ask" && b.browser_sites.is_empty());
            if !safe {
                let _ = crate::catalog::uninstall(app, &c.id).await;
                return Err(fail(&made, format!("«{}» поставился не с безопасными правами: снят", c.name)));
            }
        }
        made.push((reply["bot"]["name"].as_str().unwrap_or(&c.name).to_string(), id));
    }

    let mut member_ids: Vec<String> = existing_room.as_ref().map(|r| r.bot_ids.clone()).unwrap_or_default();
    let mut newcomers: Vec<String> = plan.add_bots.iter().map(|b| b.id.clone()).collect();
    newcomers.extend(made.iter().map(|(_, id)| id.clone()));
    for id in newcomers {
        if !member_ids.contains(&id) {
            member_ids.push(id);
        }
    }
    let mut room_text = String::new();
    if let Some(room_plan) = &plan.room {
        // Ведущий команды оркестратора — сам оркестратор: его карточка ведущего не меняет.
        let lead_id = if plan.lead.is_empty() || plan.host_orchestrator {
            None
        } else {
            let everyone = app.store.bots();
            member_ids.iter().find(|id| everyone.iter().any(|b| &b.id == *id && same(&b.name, &plan.lead))).cloned()
        };
        if room_plan.is_new {
            let mut input = json!({ "name": room_plan.name, "botIds": member_ids });
            if let Some(lead) = &lead_id {
                input["lead"] = json!(lead);
            }
            let room = app.store.add_room(&input).await.map_err(|e| fail(&made, e.to_string()))?;
            room_text = format!(" Команда «{}» создана ({}).", room.name, room.id);
        } else {
            // Только состав и ведущий: папки и навыки команды не трогаем.
            let mut patch = json!({ "botIds": member_ids });
            if let Some(lead) = &lead_id {
                patch["lead"] = json!(lead);
            }
            app.store.update_room(&room_plan.id, &patch).await.map_err(|e| fail(&made, e.to_string()))?;
            if plan.host_orchestrator {
                app.store.set_room_stage(&room_plan.id, crate::store::STAGE_READY).await.map_err(|e| fail(&made, e.to_string()))?;
            }
            room_text = format!(" Команда «{}» обновлена.", room_plan.name);
        }
    }
    let created = if made.is_empty() { "новых ботов нет".to_string() } else { made.iter().map(|(n, i)| format!("{n} ({i})")).collect::<Vec<_>>().join(", ") };
    Ok(format!("Человек согласился. Создано: {created}.{room_text}"))
}

// ---------------------------------------------------------------------------
// search_catalog
// ---------------------------------------------------------------------------

/// Поиск по каталогу готовых ботов и навыков: только чтение. Совпадения считаются по названию, описанию и роли.
pub async fn search_catalog(app: &Arc<App>, args: &Value) -> Result<String, String> {
    let query = args.get("query").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let words: Vec<String> = query.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_string).collect();
    if words.is_empty() {
        return Err("нужен query: слова задачи, например «таблицы отчёты»".into());
    }
    let kind = args.get("kind").and_then(Value::as_str).filter(|k| *k == "agent" || *k == "skill");
    let mut scored: Vec<(usize, &crate::catalog::Item)> = crate::catalog::catalog()
        .items
        .iter()
        .filter(|i| matches!(i.kind.as_str(), "agent" | "skill") && kind.is_none_or(|k| k == i.kind))
        .filter_map(|item| {
            let m = &item.manifest;
            let head = [crate::catalog::loc(&m["title"], "ru"), crate::catalog::loc(&m["title"], "en"), item.id.clone()].join(" ").to_lowercase();
            let body = [crate::catalog::loc(&m["summary"], "ru"), crate::catalog::loc(&m["summary"], "en"), crate::catalog::loc(&m["agent"]["role"], "ru")].join(" ").to_lowercase();
            // Корень слова: «таблицы» находит «таблица», окончания не мешают.
            let score: usize = words
                .iter()
                .map(|w| {
                    let count = w.chars().count();
                    let stem: String = if count > 5 { w.chars().take(count - 1).collect() } else { w.clone() };
                    usize::from(head.contains(&stem)) * 3 + usize::from(body.contains(&stem))
                })
                .sum();
            (score > 0).then_some((score, item))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    if scored.is_empty() {
        return Ok("В каталоге ничего не нашлось. Попробуй другие слова или предложи нового бота через propose_team.".into());
    }
    let mut lines = Vec::new();
    for (_, item) in scored.into_iter().take(SEARCH_LIMIT) {
        let m = &item.manifest;
        let state = crate::catalog::state_of(app, item).await;
        let installed = if state["installed"].as_bool().unwrap_or(false) { "установлен" } else { "не установлен" };
        let summary: String = clean_line(&crate::catalog::loc(&m["summary"], "ru")).chars().take(140).collect();
        let role = if item.kind == "agent" { format!(" — {}", clean_line(&crate::catalog::loc(&m["agent"]["role"], "ru")).chars().take(MAX_ROLE).collect::<String>()) } else { String::new() };
        let needs = if item.kind == "agent" { catalog_needs(item) } else { Vec::new() };
        let needs = if needs.is_empty() { "ничего особого".to_string() } else { needs.join("; ") };
        lines.push(format!("{} [{}] {}{role}. {summary} Нужно: {needs}. {installed}.", item.id, item.kind, clean_line(&crate::catalog::loc(&m["title"], "ru"))));
    }
    Ok(format!("{}\n\nБотов из списка (kind agent) можно предложить человеку через propose_team, поле catalogBots: [id].", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offers::Offers;

    #[test]
    fn instructions_refuse_zero_width_text() {
        for bad in ['\u{200B}', '\u{2060}', '\u{FEFF}', '\u{00AD}', '\u{202E}', '\u{200E}', '\u{E0041}'] {
            assert!(hidden_in_instructions(bad), "{:x} пропущен", bad as u32);
        }
        // Соединители эмодзи и обычный текст допустимы.
        for good in ['\u{200D}', '\u{200C}', 'я', 'a', '\n', '👍'] {
            assert!(!hidden_in_instructions(good), "{:x} отвергнут", good as u32);
        }
    }

    fn plan(name: &str) -> TeamPlan {
        TeamPlan {
            new_bots: vec![NewBot { name: name.into(), role: "r".into(), instructions: "i".into() }],
            catalog_bots: vec![],
            add_bots: vec![],
            room: Some(RoomPlan { id: String::new(), name: "К".into(), is_new: true }),
            lead: String::new(),
            access: vec![],
            access_key: String::new(),
            host_name: "Хозяин".into(),
            host_driver: "api".into(),
            host_model: "m".into(),
            host_extra: vec![],
            host_orchestrator: false,
        }
    }

    fn offer(id: &str, plan: TeamPlan) -> Offer {
        Offer { id: id.into(), host_bot_id: "host".into(), specialist_id: String::new(), reason: String::new(), task: String::new(), at: 1, team: Some(plan), change: None }
    }

    #[test]
    fn program_folder_lines_are_flagged_for_the_card() {
        let mut p = plan("Автор");
        p.access = vec![format!("папки: C:\\bin ({PROGRAM_FOLDER_MARK}: из неё можно запускать код)"), "сменит ведущего".into()];
        let event = p.event_json(&offer("o", p.clone()));
        assert_eq!(event["accessWarn"], json!([true, false]));
    }

    #[test]
    fn the_same_proposal_finds_the_pending_card_and_a_different_one_does_not() {
        let offers = Offers::default();
        offers.add(offer("offer_1", plan("Автор")));
        assert_eq!(offers.find_team("host", &plan("Автор")).as_deref(), Some("offer_1"), "тот же состав — та же карточка");
        assert!(offers.find_team("host", &plan("Другой")).is_none());
        assert!(offers.find_team("other", &plan("Автор")).is_none(), "карточки чужого бота не видны");
        offers.take("host", "offer_1");
        assert!(offers.find_team("host", &plan("Автор")).is_none(), "решённая карточка не мешает новой");
    }

    #[tokio::test]
    async fn a_decision_reaches_every_waiter() {
        let offers = Offers::default();
        let mut first = offers.open_decision("o");
        let mut second = offers.open_decision("o");
        offers.decide("o", "готово".into());
        assert_eq!(first.wait_for(|v| v.is_some()).await.unwrap().clone().as_deref(), Some("готово"));
        assert_eq!(second.wait_for(|v| v.is_some()).await.unwrap().clone().as_deref(), Some("готово"));
    }
}
