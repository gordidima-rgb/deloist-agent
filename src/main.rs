//! Deloist — команда AI-ботов в отдельном приложении.
//! Без аргументов открывает окно и живёт в трее; с `--server` работает без окна.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(deloist_private)]
#[path = "../private/src/account.rs"]
mod account;
#[cfg(not(deloist_private))]
#[path = "account_stub.rs"]
mod account;
#[cfg(deloist_private)]
#[path = "../private/src/relay.rs"]
mod relay;
#[cfg(not(deloist_private))]
#[path = "relay_stub.rs"]
mod relay;
mod agents_md;
mod brain;
mod agent_client;
mod browser;
mod browser_tools;
mod catalog;
mod changelog;
#[cfg(feature = "desktop")]
mod capture;
mod cloud;
mod instabox;
mod cdp;
mod compact;
mod connectors;
mod connectors_client;
mod context;
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(not(feature = "desktop"))]
#[path = "desktop_stub.rs"]
mod desktop;
mod drivers;
mod episodes;
mod extract;
#[cfg(deloist_private)]
#[path = "../private/src/feedback.rs"]
mod feedback;
#[cfg(not(deloist_private))]
#[path = "feedback_stub.rs"]
mod feedback;
mod fetch;
mod goal_tools;
mod goals;
mod grants;
mod habits;
mod help;
mod graph;
mod guard;
mod headless;
mod hermes_import;
#[cfg(feature = "desktop")]
mod hotkeys;
#[cfg(feature = "desktop")]
mod icon_art;
mod image;
mod intro;
mod keys;
mod learn_rules;
mod know_me;
mod ledger;
mod limit;
mod mail;
mod mail_net;
mod history;
mod watch;
mod memory;
mod messengers;
mod oauth;
mod native_host;
mod pipe_auth;
mod mcp;
mod mcp_server;
mod moli;
mod obscura;
mod offers;
mod optimizer;
#[cfg(all(windows, feature = "desktop"))]
mod pets;
#[cfg(not(all(windows, feature = "desktop")))]
#[path = "pets_stub.rs"]
mod pets;
mod dates_ru;
mod describe;
#[cfg(all(windows, feature = "desktop"))]
mod dock;
#[cfg(not(all(windows, feature = "desktop")))]
#[path = "dock_stub.rs"]
mod dock;
mod attach;
mod process;
mod providers;
mod openrouter_login;
mod queue;
mod reach;
mod rights;
mod routine_tools;
mod rooms;
mod runs;
mod runtime;
mod runtime_node;
#[cfg(windows)]
mod sandbox;
#[cfg(not(windows))]
#[path = "sandbox_other.rs"]
mod sandbox;
mod screen;
mod selfdev;
mod schedule;
mod scheduler;
mod scripts;
mod search;
mod secrets;
mod sensitive;
mod shellplan;
#[cfg(feature = "desktop")]
mod selection;
mod server;
mod service;
mod site_tips;
mod skills;
mod stats;
mod store;
mod stt_local;
mod system;
mod teach;
mod discord;
mod password_vault;
mod slack;
mod telegram;
mod reminders;
mod toast_reply;
mod aumid;
mod board;
mod team;
mod tg_channels;
mod team_changes;
mod team_import_requests;
mod team_proposal;
mod team_tasks;
mod transfer;
mod workflow_api;
mod workflow_io;
mod workflow_run;
mod workflow_tools;
mod workflow_walk;
mod workflows;
mod tools;
mod todo_parse;
mod todos;
mod turn;
mod update;
mod user_chrome;
mod vault;
mod web_search;
#[cfg(deloist_private)]
#[path = "../private/src/voice_cloud.rs"]
mod voice_cloud;
#[cfg(not(deloist_private))]
#[path = "voice_cloud_stub.rs"]
mod voice_cloud;
mod voice;
mod webview_loader;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

pub struct App {
    pub store: store::Store,
    pub turns: turn::Turns,
    pub queues: queue::Queues,
    pub browsers: browser::Browsers,
    pub pets: pets::Pets,
    /// Док агентов у края экрана: отдельный процесс, пока включён.
    pub dock: dock::Dock,
    pub ledger: ledger::Ledger,
    pub updates: update::Updates,
    pub mcp: mcp::Mcp,
    pub rooms: rooms::Rooms,
    /// Сколько картинок бот уже нарисовал сегодня.
    pub images: image::Images,
    /// Вход в аккаунт Deloist.
    pub account: account::Account,
    /// «Подключить свою ИИ»: агенты из реестра ACP и провайдеры по ключу.
    pub providers: Arc<providers::Providers>,
    /// Цели ботов и самопроверки.
    pub goals: goals::Goals,
    /// Привычки человека и его отметки по дням.
    pub habits: habits::Habits,
    /// Задачи человека: срок, раздел, повторы, чек-лист и архив.
    pub todos: todos::Todos,
    /// Что поставлено из каталога: навыки, агенты, приложения.
    pub catalog: catalog::Installs,
    /// Подключения из каталога: секреты, входы OAuth, последние ошибки.
    pub connectors: connectors::Connectors,
    /// Облако на машине Instabox: вход, создание, состояние (только из окна).
    pub instabox: instabox::Hub,
    /// Мост в Telegram: свой бот человека и привязанный чат.
    pub telegram: telegram::Telegram,
    /// Нерешённые карточки «Подключу специалиста».
    pub offers: offers::Offers,
    /// Мост в Discord: свой бот человека и привязанная личная переписка.
    pub discord: discord::Discord,
    pub relay: relay::Relay,
    /// Мост в Slack: своё приложение человека и привязанная личная переписка.
    pub slack: slack::Slack,
    /// Менеджер паролей: ключ сессии Bitwarden только в памяти.
    pub vault: password_vault::Vault,
    /// Задачи, которые боты команды дали друг другу без ожидания.
    pub tasks: team_tasks::Tasks,
    /// Заявки командной строки на добавление команды из файла: ждут человека в окне.
    pub import_requests: team_import_requests::ImportRequests,
    /// След переноса ботов между доверенными машинами: ACL и происхождение при сбое не теряются.
    pub transfer: transfer::Transfer,
    /// Рабочие процессы: графы шагов, запуски и согласия.
    pub workflows: workflows::Workflows,
    /// Почта ботов: ящики IMAP+SMTP, триггеры рутин, авторабота.
    pub mail: mail::Mail,
    /// Журнал запусков: что бот делал и как продолжить после перезапуска.
    pub runs: runs::Runs,
    /// Модели, которые отдыхают после лимита: общий счёт на компьютер.
    pub rest: limit::Rest,
    /// Сторож страниц: триггеры рутин «на странице новые записи».
    pub watch: watch::Watch,
}

/// Папка приложения: при разработке корень репозитория, иначе папка рядом с exe.
fn app_root() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("deloist"));
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    dir.ancestors()
        .find(|p| p.join("Cargo.toml").exists())
        .map(Path::to_path_buf)
        .unwrap_or(dir)
}

fn read_dotenv(root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join(".env")) else {
        return out;
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            out.insert(key.trim().to_string(), value.to_string());
        }
    }
    out
}

fn data_dir(root: &Path) -> PathBuf {
    std::env::var_os("CREW_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("data"))
}

/// Одна папка данных — одна работающая копия: две записывали бы state.json вперемешку.
/// Система снимает блокировку сама, даже если процесс упал. `Ok(None)` значит «занято».
fn lock_data_dir(dir: &Path) -> Result<Option<std::fs::File>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Не смог создать {}: {e}", dir.display()))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("crew.lock"))
        .map_err(|e| format!("Не смог открыть блокировку в {}: {e}", dir.display()))?;
    // После обновления прежняя копия ещё закрывается: ждём её, а не считаем программу запущенной.
    let attempts = if std::env::args().any(|a| a == "--after-update") { 80 } else { 1 };
    for attempt in 0..attempts {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => {
                if attempt + 1 < attempts {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("Не смог взять блокировку: {e}")),
        }
    }
    Ok(None)
}

async fn boot(root: &Path) -> std::io::Result<Arc<App>> {
    let store = store::Store::open(data_dir(root)).await?;
    store.set_env_file(read_dotenv(root));
    let ledger = ledger::Ledger::open(store.data_dir().to_path_buf()).await;
    let data = store.data_dir().to_path_buf();
    let turns = turn::Turns::open(store.data_dir());
    let queues = queue::Queues::open(store.data_dir().to_path_buf(), turns.sender()).await;
    let app = Arc::new(App { store, turns, queues, browsers: browser::Browsers::default(), pets: pets::Pets::default(), dock: dock::Dock::default(), ledger, updates: update::Updates::default(), mcp: mcp::Mcp::default(), rooms: rooms::Rooms::default(), images: image::Images::default(), account: account::Account::default(), providers: Arc::default(), goals: goals::Goals::open(data.clone()).await, habits: habits::Habits::open(data.clone()), todos: todos::Todos::open(data.clone()),catalog: catalog::Installs::default(), connectors: connectors::Connectors::open(&data), telegram: telegram::Telegram::default(), offers: offers::Offers::open(&data), discord: discord::Discord::default() , instabox: instabox::Hub::default(), relay: relay::Relay::default(), slack: slack::Slack::default(), vault: password_vault::Vault::default(), tasks: team_tasks::Tasks::open(&data), import_requests: team_import_requests::ImportRequests::default(), transfer: transfer::Transfer::open(&data), workflows: workflows::Workflows::open(&data), runs: runs::Runs::open(&data), rest: limit::Rest::open(&data), mail: mail::Mail::open(&data), watch: watch::Watch::open(&data) });
    // До первого хода: отметки задач у новой версии начинаются с этой минуты.
    extract::init(&app).await;
    // 0.4.3: правила из инструкций — в brain.md, MAIN.md — в memory.md (тот при первом чтении памяти).
    brain::migrate_all(&app).await;
    // Незаконченные ходы продолжаются или ждут человека до расписания и очередей.
    runs::recover(&app).await;
    scheduler::start(app.clone());
    providers::start(app.clone());
    update::start(app.clone());
    telegram::start(app.clone());
    discord::start(app.clone());
    relay::start(app.clone());
    // Режим сервера после перезагрузки или обновления: сон снова под запретом, связь с сайтом уже запущена выше без окна.
    if app.store.settings().server_mode && system::server_mode_supported() {
        system::sleep_lock().set(true);
    }
    slack::start(app.clone());
    password_vault::start(app.clone());
    team_tasks::resume(&app);
    Ok(app)
}

fn bind_address(default_port: u16, cloud: bool) -> Result<SocketAddr, String> {
    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let ip: IpAddr = host.parse().map_err(|_| format!("HOST должен быть IP-адресом: {host}"))?;
    // Сервер управляет командами на этой машине, в сеть его отдавать нельзя. Исключение одно:
    // облачный режим (контейнер за шлюзом), где вход проверяется подписью шлюза.
    if !ip.is_loopback() && !cloud {
        return Err(format!("Deloist работает только на этой машине, HOST={host} не подходит"));
    }
    let port = match std::env::var("PORT") {
        Ok(p) => p.parse().map_err(|_| format!("PORT должен быть числом: {p}"))?,
        Err(_) => default_port,
    };
    Ok(SocketAddr::new(ip, port))
}

/// Паспорт живой копии: по нему командная строка и MCP-сервер находят приложение.
fn publish(data_dir: &Path, port: u16) -> String {
    let token = guard::random_token();
    let published = runtime::Runtime {
        port,
        token: token.clone(),
        pid: std::process::id(),
        started_at: store::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if let Err(e) = runtime::write(data_dir, &published) {
        eprintln!("[runtime] не смог записать паспорт копии: {e}");
    }
    token
}

fn run_server(root: &Path) -> Result<(), String> {
    // Облачный режим проверяется первым: без ключа шлюза и адреса программа не стартует вовсе.
    let cloud = guard::CloudConfig::from_env()?;
    // Политика облака должна действовать уже при загрузке (MCP, подключения, боты), а не после неё.
    guard::set_cloud(cloud.is_some());
    let Some(_lock) = lock_data_dir(&data_dir(root))? else {
        return Err("Эта папка данных уже используется другой копией Deloist.".into());
    };
    update::clean_leftovers();
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let app = boot(root).await.map_err(|e| e.to_string())?;
        let addr = bind_address(8899, cloud.is_some())?;
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("Не смог занять {addr}: {e}"))?;
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let boot = std::env::var("CREW_BOOT_TOKEN").unwrap_or_else(|_| guard::random_token());
        let agent = publish(app.store.data_dir(), addr.port());
        user_chrome::start(app.clone(), agent.clone());
        let data_dir = app.store.data_dir().to_path_buf();
        // Тосты с ответом — только на компьютере человека; у облачной копии рабочего стола нет.
        if cloud.is_none() {
            toast_reply::start(app.clone(), tokio::runtime::Handle::current());
        }
        let mut guard = guard::Guard::new(addr.port(), boot.clone(), agent);
        if let Some(config) = cloud {
            // Облако: ссылки со входом в журнал не печатаем, вход только через шлюз.
            guard::set_cloud(true);
            guard::set_public_origin(config.origin());
            guard = guard.with_cloud(config);
            println!("deloist (облачный режим)  →  {addr}");
        } else {
            println!("deloist  →  http://{addr}/?boot={boot}");
            println!("ссылка одноразовая");
        }
        println!("данные →  {}", app.store.data_dir().display());
        let guard = Arc::new(guard);
        let served = server::serve(app, listener, guard, CancellationToken::new()).await.map_err(|e| e.to_string());
        runtime::remove(&data_dir);
        served
    })
}

#[cfg(feature = "desktop")]
const DEFAULT_PORT: u16 = 18899;

#[cfg(not(feature = "desktop"))]
fn run_desktop(_root: &Path) -> Result<(), String> {
    Err("Эта сборка без окна: запусти `deloist --server`.".into())
}

#[cfg(feature = "desktop")]
fn run_desktop(root: &Path) -> Result<(), String> {
    let Some(_lock) = lock_data_dir(&data_dir(root))? else {
        if desktop::activate_running_window() {
            return Ok(());
        }
        return Err("Deloist уже запущен: значок ищи в трее.".into());
    };
    update::clean_leftovers();
    let runtime = Arc::new(tokio::runtime::Runtime::new().map_err(|e| e.to_string())?);
    let shutdown = CancellationToken::new();

    let (app, listener) = runtime.block_on(async {
        let app = boot(root).await.map_err(|e| e.to_string())?;
        // Порт постоянный: на нём держатся адреса вебхуков и настройки окна, у браузера они привязаны к адресу.
        // Занят другой программой: берём свободный и запоминаем его.
        let saved = app.store.settings().local_port;
        let wanted = if saved > 0 { saved as u16 } else { DEFAULT_PORT };
        let listener = match tokio::net::TcpListener::bind(bind_address(wanted, false)?).await {
            Ok(listener) => listener,
            Err(_) => tokio::net::TcpListener::bind(bind_address(0, false)?)
                .await
                .map_err(|e| format!("Не смог открыть локальный порт: {e}"))?,
        };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let _ = app.store.remember_port(port).await;
        // Первый запуск окна: ярлык на рабочий стол, один раз. Отладочная сборка настоящий стол не трогает.
        if app.store.take_first_shortcut().await && !cfg!(debug_assertions) {
            std::thread::spawn(|| {
                if let Err(e) = system::create_desktop_shortcut() {
                    eprintln!("[shortcut] {e}");
                }
            });
        }
        Ok::<_, String>((app, listener))
    })?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let origin = format!("http://{addr}");
    let webview_data = app.store.data_dir().join("webview");
    let data_dir = app.store.data_dir().to_path_buf();
    let agent = publish(&data_dir, addr.port());
    {
        let (app, agent) = (app.clone(), agent.clone());
        // Канал для расширения «Мой Chrome»: после паспорта копии, ключ тот же.
        let _guard = runtime.enter();
        user_chrome::start(app, agent);
    }
    let english = app.store.settings().ui_lang == "en";
    let guard = Arc::new(guard::Guard::new(addr.port(), guard::random_token(), agent));
    let for_exit = app.clone();

    // Питомцы — отдельный процесс: адрес со одноразовым входом и сторож сервера живут в App.
    app.pets.attach(origin.clone(), guard.clone());
    app.dock.attach(origin.clone(), guard.clone());
    selection::attach(origin.clone(), guard.clone());
    hotkeys::start(app.clone(), runtime.handle().clone());
    toast_reply::start(app.clone(), runtime.handle().clone());

    {
        let shutdown = shutdown.clone();
        let guard = guard.clone();
        runtime.spawn(async move {
            // Сначала питомцы, потом сервер: процесс успеет открыть WebView, когда порт уже слушает.
            pets::sync_with_store(&app);
            dock::sync_with_store(&app);
            if let Err(e) = server::serve(app, listener, guard, shutdown).await {
                eprintln!("[server] {e}");
            }
        });
    }

    let entry = {
        let origin = origin.clone();
        move || format!("{origin}/?boot={}", guard.issue_boot())
    };
    let stop = shutdown.clone();
    let exit_dir = data_dir.clone();
    let on_exit = move || {
        // Хвостов быть не должно: сначала команды в песочнице и питомцы, потом браузеры.
        sandbox::shutdown();
        for_exit.pets.stop();
        for_exit.dock.stop();
        // Браузеры закрываем штатно: они успеют записать cookies, и вход не пропадёт.
        runtime.block_on(for_exit.browsers.close_all());
        runtime.block_on(mcp::shutdown(&for_exit));
        drivers::acp::shutdown();
        runtime::remove(&exit_dir);
        stop.cancel();
    };
    // Автозапуск стартует в трее: окно с WebView не нужно, пока человек его не открыл.
    let hidden = std::env::args().any(|a| a == "--tray");
    desktop::run(entry, origin, webview_data, hidden, english, on_exit)
}

fn first_arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == name {
            return args.next();
        }
    }
    None
}

/// Сборка для людей не открывает консоль (иначе окно мелькало бы при каждом запуске), поэтому
/// команде нужно подключиться к консоли того, кто её вызвал. Без этого вывод уходит в никуда.
/// Перенаправленный вывод (канал, файл) остаётся как был: вызов его не трогает.
fn attach_parent_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn main() {
    // Один признак облака: `CREW_CLOUD=1` без `--server` не тихий режим ПК, а ошибка запуска (до всего остального).
    if std::env::var("CREW_CLOUD").is_ok_and(|v| v == "1") && !std::env::args().any(|a| a == "--server") {
        eprintln!("CREW_CLOUD=1 без --server: облачный режим запускается только как сервер, запуск отменён");
        std::process::exit(1);
    }
    // Первым делом, до рантайма tokio, потоков и дочерних процессов: в облачном режиме ключ
    // шифрования секретов приходит в `CREW_SECRETS_KEY`, и его надо убрать из окружения, пока
    // его никто не унаследовал: команды ботов и агенты получают окружение процесса.
    if let Err(message) = secrets::init_at_startup(&std::env::args().skip(1).collect::<Vec<_>>()) {
        eprintln!("{message}");
        std::process::exit(1);
    }
    // Окно и уведомления подписываются «Deloist», если ярлык «Пуск» несёт тот же идентификатор (aumid.rs).
    #[cfg(windows)]
    if aumid::in_use() {
        aumid::set_process_app_id();
    }
    // Распознаватель речи — отдельный процесс без окна: модель живёт в нём (stt_local.rs).
    if let Some(spec) = first_arg("--stt-worker") {
        std::process::exit(stt_local::run_worker(&spec));
    }
    let root = app_root();
    // Питомцы — отдельный процесс той же программы; главное окно он не трогает.
    if let Some(mode) = first_arg("--pets") {
        if let Err(message) = pets::run(&mode, first_arg("--bot"), first_arg("--at"), first_arg("--slot")) {
            eprintln!("{message}");
        }
        return;
    }
    // Док агентов — тоже отдельный процесс со своим окном у края экрана.
    if let Some(layer) = first_arg("--dock") {
        if let Err(message) = dock::run(&layer, first_arg("--edge"), first_arg("--display")) {
            eprintln!("{message}");
        }
        return;
    }
    // Подсказка «правка выделенного» — тоже отдельный процесс, маленькое окно у курсора.
    #[cfg(feature = "desktop")]
    if let Some(at) = first_arg("--prompt") {
        if let Err(message) = selection::run_prompt(&at, first_arg("--target"), first_arg("--mode")) {
            eprintln!("{message}");
        }
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Хост Native Messaging: так программу запускает Chrome для расширения «Мой Chrome».
    if native_host::is_host_call(&args) {
        std::process::exit(native_host::run(&data_dir(&root), &args));
    }

    // MCP-сервер и командная строка работают поверх уже запущенного приложения.
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(mcp_server::run(&root, first_arg("--browser-bot")));
    }
    if let Some(command) = headless::parse(&args) {
        attach_parent_console();
        std::process::exit(headless::run(&root, command));
    }

    let headless = args.iter().any(|a| a == "--server");
    let result = if headless { run_server(&root) } else { run_desktop(&root) };
    if let Err(message) = result {
        if headless {
            eprintln!("{message}");
        } else {
            desktop::show_error(&message);
        }
        std::process::exit(1);
    }
}

