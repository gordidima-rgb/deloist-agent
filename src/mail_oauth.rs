//! Вход в Яндекс Почту через OAuth 2.0 + PKCE S256, без секрета клиента.
//!
//! Схема (yandex.ru/dev/id/doc/ru/codes/code-url, yandex.ru/support/mail-business/web/security/oauth.html):
//! браузер открывает `oauth.yandex.ru/authorize` с `code_challenge`, возврат — на `http://127.0.0.1:<порт>/yandex-mail`,
//! код меняется на токены запросом с `code_verifier`; IMAP и SMTP принимают токен по SASL XOAUTH2 (`mail_net`).
//! Токены лежат только в зашифрованных секретах ящика (`mail.<id>`), не в `mail.json` и не в ответах API.
//! Кнопка входа есть, только когда в `mail::YANDEX_CLIENT_ID` вписан client id приложения.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::oauth::{self, Config, Tokens};
use crate::secrets::Secrets;
use crate::App;

const AUTHORIZE_URL: &str = "https://oauth.yandex.ru/authorize";
pub const TOKEN_URL: &str = "https://oauth.yandex.ru/token";
const SCOPE: &str = "login:email mail:imap_full mail:smtp";
const INFO_URL: &str = "https://login.yandex.ru/info?format=json";
/// Зарегистрирован в приложении Яндекса с этим портом; при неточном совпадении `redirect_uri` Яндекс игнорирует параметр (справка code-url),
/// поэтому порт один и тот же, запасного случайного нет.
const PORT: u16 = 47831;
const REDIRECT_URI: &str = "http://127.0.0.1:47831/yandex-mail";
const CALLBACK_PATH: &str = "/yandex-mail";
const WAIT: Duration = Duration::from_secs(600);
/// Готовый вход, который не завершили, не держит токены в памяти дольше этого.
const READY_TTL_MS: i64 = 15 * 60 * 1000;
/// Хосты из справки OAuth Яндекса (imap.yandex.com / smtp.yandex.com).
pub const IMAP_HOST: &str = "imap.yandex.com";
pub const SMTP_HOST: &str = "smtp.yandex.com";

/// Одновременное обновление токена одним ходом: два запроса не тратят один refresh-токен дважды.
static REFRESH_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn enabled() -> bool {
    !crate::mail::YANDEX_CLIENT_ID.is_empty()
}

enum Outcome {
    Waiting,
    /// Токены, адрес и время получения (мс).
    Ready(Tokens, String, i64),
    Failed(String),
}

/// Вход в процессе: один на программу, новый «Войти через Яндекс» гасит прежний (его слушатель закрывается в `Drop`).
/// Токены ждут здесь, в памяти, пока человек не завершит подключение, но не дольше `READY_TTL_MS`.
pub struct Flow {
    id: String,
    outcome: Arc<Mutex<Outcome>>,
    task: tokio::task::AbortHandle,
}

impl Drop for Flow {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Адрес входа.
pub fn authorize_url(client_id: &str, challenge: &str, state: &str) -> String {
    let mut url = reqwest::Url::parse(AUTHORIZE_URL).expect("постоянный адрес");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    url.into()
}

async fn exchange(token_url: &str, client_id: &str, code: &str, verifier: &str) -> Result<Tokens, String> {
    let cfg = Config { token_url: token_url.into(), client_id: client_id.into(), ..Config::default() };
    let form = vec![("grant_type", "authorization_code".to_string()), ("code", code.to_string()), ("code_verifier", verifier.to_string())];
    oauth::token_request(&cfg, "", form).await.map_err(|e| format!("Яндекс не выдал токен: {}", e.text))
}

/// Адрес ящика по токену: `default_email` из login.yandex.ru/info.
async fn fetch_email(info_url: &str, token: &str) -> Result<String, String> {
    let response = oauth::http().get(info_url).header("Authorization", format!("OAuth {token}")).send().await.map_err(|_| "Яндекс не ответил: проверьте интернет".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Яндекс не назвал адрес ящика (код {})", response.status().as_u16()));
    }
    let info: Value = response.json().await.map_err(|_| "Яндекс ответил не так, как ожидалось".to_string())?;
    let address = info["default_email"].as_str().unwrap_or("").trim().to_lowercase();
    if crate::mail_net::valid_address(&address) { Ok(address) } else { Err("Яндекс не назвал адрес ящика".into()) }
}

/// `POST /api/mail/oauth/yandex/start`: открыть браузер и ждать возврата. Результат — через `status`.
pub async fn start(app: &Arc<App>) -> Result<Value, String> {
    let (id, url) = begin(app).await?;
    crate::desktop::open_in_browser(&url);
    Ok(json!({ "flow": id }))
}

/// Занять порт, запустить ожидание возврата; номер входа и адрес для браузера.
async fn begin(app: &Arc<App>) -> Result<(String, String), String> {
    if !enabled() {
        return Err("вход через Яндекс не настроен в этой сборке".into());
    }
    if crate::cloud::active() {
        return Err("в облаке вход через браузер недоступен: создайте пароль приложения".into());
    }
    // Прежний вход гасим первым: он держит порт. Слушатель закрывается, когда рантайм дойдёт до отмены, поэтому занимаем порт с повторами.
    drop(app.mail.oauth.lock().unwrap_or_else(|e| e.into_inner()).take());
    let listener = bind_port().await?;
    let verifier = oauth::new_verifier();
    let state = crate::guard::random_token();
    let url = authorize_url(crate::mail::YANDEX_CLIENT_ID, &oauth::challenge(&verifier), &state);
    let outcome = Arc::new(Mutex::new(Outcome::Waiting));
    let en = app.store.settings().ui_lang == "en";
    let slot = outcome.clone();
    let task = tokio::spawn(async move {
        let code = match tokio::time::timeout(WAIT, crate::openrouter_login::wait_for_code(en, vec![Some(listener)], &state, CALLBACK_PATH)).await {
            Err(_) => Err("вход не завершён за 10 минут".to_string()),
            Ok(result) => result,
        };
        let next = match code {
            Ok(code) => match exchange(TOKEN_URL, crate::mail::YANDEX_CLIENT_ID, &code, &verifier).await {
                Ok(tokens) => match fetch_email(INFO_URL, &tokens.access).await {
                    Ok(address) => Outcome::Ready(tokens, address, crate::store::now()),
                    Err(e) => Outcome::Failed(e),
                },
                Err(e) => Outcome::Failed(e),
            },
            Err(e) => Outcome::Failed(e),
        };
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = next;
    });
    let id = crate::store::uid("oa");
    *app.mail.oauth.lock().unwrap_or_else(|e| e.into_inner()) = Some(Flow { id: id.clone(), outcome, task: task.abort_handle() });
    Ok((id, url))
}

#[cfg(windows)]
fn bind_exclusive() -> Result<tokio::net::TcpListener, String> {
    use std::mem;
    use std::os::windows::io::{FromRawSocket, RawSocket};
    use windows_sys::Win32::Networking::WinSock::*;
    unsafe {
        // Winsock может быть ещё не поднят в этом процессе: вызов со счётчиком, повторы безвредны.
        let mut wsa: WSADATA = mem::zeroed();
        if WSAStartup(0x202, &mut wsa) != 0 {
            return Err("winsock failed".into());
        }
        // Как в std: без наследования, иначе порт входа унаследуют программы, которые запускают боты.
        let sock = WSASocketW(AF_INET as i32, SOCK_STREAM, IPPROTO_TCP, std::ptr::null(), 0, WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT);
        if sock == INVALID_SOCKET {
            return Err("socket failed".into());
        }
        let opt: i32 = 1;
        if setsockopt(sock, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, &opt as *const i32 as *const u8, mem::size_of::<i32>() as i32) != 0 {
            closesocket(sock);
            return Err("setsockopt failed".into());
        }
        let mut addr: SOCKADDR_IN = mem::zeroed();
        addr.sin_family = AF_INET;
        addr.sin_port = htons(PORT);
        addr.sin_addr.S_un.S_addr = inet_addr(b"127.0.0.1\0".as_ptr());
        if bind(sock, &addr as *const SOCKADDR_IN as *const SOCKADDR, mem::size_of::<SOCKADDR_IN>() as i32) != 0 {
            closesocket(sock);
            return Err("bind failed".into());
        }
        if listen(sock, 16) != 0 {
            closesocket(sock);
            return Err("listen failed".into());
        }
        let std_listener = <std::net::TcpListener as FromRawSocket>::from_raw_socket(sock as RawSocket);
        std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        tokio::net::TcpListener::from_std(std_listener).map_err(|e| e.to_string())
    }
}

async fn bind_port() -> Result<tokio::net::TcpListener, String> {
    let mut last: Option<String> = None;
    for _ in 0..6 {
        #[cfg(windows)]
        {
            match bind_exclusive() {
                Ok(listener) => return Ok(listener),
                Err(e) => last = Some(e),
            }
        }
        #[cfg(not(windows))]
        {
            match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
                Ok(listener) => return Ok(listener),
                Err(e) => last = Some(e.to_string()),
            }
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    drop(last);
    Err("не удалось занять порт 47831 на этом компьютере (его держит другая программа): закройте её или создайте пароль приложения".into())
}

const EXPIRED: &str = "вход в Яндекс устарел: начните заново";

/// Готовый вход старше `READY_TTL_MS` — вход закрыт, токены забыты. Возвращает, закрыли ли.
fn drop_if_stale(slot: &mut Option<Flow>) -> bool {
    let stale = slot.as_ref().is_some_and(|f| matches!(&*f.outcome.lock().unwrap_or_else(|e| e.into_inner()), Outcome::Ready(_, _, at) if crate::store::now() - at > READY_TTL_MS));
    if stale {
        *slot = None;
    }
    stale
}

/// `GET /api/mail/oauth/yandex/{flow}`: `waiting`, `ready` (можно завершать) или `failed` с текстом.
pub fn status(app: &App, id: &str) -> Option<Value> {
    let mut slot = app.mail.oauth.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|f| f.id == id)?;
    if drop_if_stale(&mut slot) {
        return Some(json!({ "status": "failed", "error": EXPIRED }));
    }
    let outcome = slot.as_ref()?.outcome.lock().unwrap_or_else(|e| e.into_inner());
    Some(match &*outcome {
        Outcome::Waiting => json!({ "status": "waiting" }),
        Outcome::Ready(_, address, _) => json!({ "status": "ready", "address": address }),
        Outcome::Failed(e) => json!({ "status": "failed", "error": e }),
    })
}

/// Забрать токены завершённого входа и адрес; вход закрывается. `None` — нет такого входа; `Err` — вход не удался или ещё идёт.
pub fn take(app: &App, id: &str) -> Option<Result<(String, Tokens), String>> {
    let mut slot = app.mail.oauth.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|f| f.id == id)?;
    if drop_if_stale(&mut slot) {
        return Some(Err(EXPIRED.into()));
    }
    let mut outcome = slot.as_ref()?.outcome.lock().unwrap_or_else(|e| e.into_inner());
    let result = match std::mem::replace(&mut *outcome, Outcome::Waiting) {
        Outcome::Ready(tokens, address, _) => Ok((address, tokens)),
        Outcome::Failed(e) => Err(e),
        Outcome::Waiting => return Some(Err("вход в Яндекс ещё не завершён".into())),
    };
    drop(outcome);
    *slot = None;
    Some(result)
}

/// Токен доступа ящика. Обновляем заранее (за минуту до конца) и по требованию (`force`: сервер отказал во входе).
/// Нет refresh-токена или Яндекс его отозвал — человеку нужно войти заново.
pub async fn fresh_token(secrets: &Secrets, scope: &str, token_url: &str, client_id: &str, force: bool) -> Result<String, String> {
    let read = || secrets.get(scope, oauth::ACCESS).filter(|t| !t.is_empty());
    let expired = || {
        let expires: i64 = secrets.get(scope, oauth::EXPIRES).and_then(|v| v.parse().ok()).unwrap_or(0);
        // Срок не записан, а обновить можно (есть refresh-токен): считаем токен просроченным, а не вечным.
        if expires == 0 { secrets.get(scope, oauth::REFRESH).is_some() } else { crate::store::now() + oauth::REFRESH_AHEAD_MS >= expires }
    };
    let before = read().ok_or("у ящика нет входа через Яндекс: войдите заново")?;
    if !force && !expired() {
        return Ok(before);
    }
    let _one_at_a_time = REFRESH_GATE.lock().await;
    // Пока ждали, токен мог обновить соседний ход.
    match read() {
        Some(now) if now != before || (!force && !expired()) => return Ok(now),
        _ => {}
    }
    let refresh = secrets.get(scope, oauth::REFRESH).ok_or("вход в Яндекс истёк: войдите заново")?;
    let cfg = Config { token_url: token_url.into(), client_id: client_id.into(), ..Config::default() };
    let form = vec![("grant_type", "refresh_token".to_string()), ("refresh_token", refresh.clone())];
    match oauth::token_request(&cfg, "", form).await {
        Ok(tokens) => {
            oauth::save_tokens(secrets, scope, &tokens, Some(&refresh))?;
            Ok(tokens.access)
        }
        Err(e) if e.revoked => Err("вход в Яндекс истёк или отозван: войдите заново".into()),
        Err(e) => Err(format!("токен Яндекса не обновился: {}", e.text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Заглушка сервера токенов: на каждый запрос отвечает следующим из `replies` (статус, тело) и считает запросы.
    async fn token_stub(replies: Vec<(u16, &'static str)>) -> (String, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}/token", listener.local_addr().unwrap().port());
        let hits = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (h, b) = (hits.clone(), bodies.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 8192];
                let mut len = 0;
                // Заголовки и тело приходят одним-двумя чтениями; ждём, пока тело дойдёт целиком.
                loop {
                    let n = socket.read(&mut buf[len..]).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    len += n;
                    let text = String::from_utf8_lossy(&buf[..len]).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let want: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
                        if body.len() >= want {
                            b.lock().unwrap().push(body.to_string());
                            break;
                        }
                    }
                }
                let n = h.fetch_add(1, Ordering::SeqCst);
                let (status, body) = replies[n.min(replies.len() - 1)];
                let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (url, hits, bodies)
    }

    fn secrets(name: &str) -> (Secrets, std::path::PathBuf) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("crew-mail-oauth-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Secrets::open(&dir), dir)
    }

    fn put(s: &Secrets, access: &str, refresh: &str, expires: i64) {
        s.set("mail.m1", oauth::ACCESS, access).unwrap();
        s.set("mail.m1", oauth::REFRESH, refresh).unwrap();
        s.set("mail.m1", oauth::EXPIRES, &expires.to_string()).unwrap();
    }

    #[test]
    fn the_sign_in_address_carries_pkce_state_scopes_and_the_loopback_redirect() {
        let url = reqwest::Url::parse(&authorize_url("cid", "chal", "st")).unwrap();
        assert_eq!(url.host_str(), Some("oauth.yandex.ru"));
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "cid");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:47831/yandex-mail");
        assert_eq!(q["scope"], "login:email mail:imap_full mail:smtp");
        assert_eq!((q["code_challenge"].as_str(), q["code_challenge_method"].as_str(), q["state"].as_str()), ("chal", "S256", "st"));
        assert!(!q.contains_key("client_secret"), "секрета клиента нет");
    }

    #[tokio::test]
    async fn a_token_that_is_still_good_is_not_refreshed_and_an_old_one_is() {
        let (s, dir) = secrets("refresh");
        let (url, hits, bodies) = token_stub(vec![(200, r#"{"access_token":"new-a","refresh_token":"r2","expires_in":3600}"#), (200, r#"{"access_token":"newer-a","expires_in":3600}"#)]).await;
        put(&s, "old-a", "r1", crate::store::now() + 3_600_000);
        assert_eq!(fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap(), "old-a");
        assert_eq!(hits.load(Ordering::SeqCst), 0, "свежий токен не обновляем");
        // Истекает через 30 секунд: пора, обновляем заранее.
        put(&s, "old-a", "r1", crate::store::now() + 30_000);
        assert_eq!(fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap(), "new-a");
        assert_eq!((s.get("mail.m1", oauth::ACCESS).as_deref(), s.get("mail.m1", oauth::REFRESH).as_deref()), (Some("new-a"), Some("r2")));
        let body = bodies.lock().unwrap()[0].clone();
        assert!(body.contains("grant_type=refresh_token") && body.contains("refresh_token=r1") && body.contains("client_id=cid"), "{body}");
        // После отказа сервера во входе — по требованию, один запрос; новый refresh не пришёл — прежний остаётся.
        assert_eq!(fresh_token(&s, "mail.m1", &url, "cid", true).await.unwrap(), "newer-a");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(s.get("mail.m1", oauth::REFRESH).as_deref(), Some("r2"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_revoked_refresh_token_asks_for_a_new_sign_in_and_a_missing_one_never_calls_the_server() {
        let (s, dir) = secrets("revoked");
        let (url, hits, _) = token_stub(vec![(400, r#"{"error":"invalid_grant"}"#)]).await;
        put(&s, "old-a", "r1", crate::store::now() - 1000);
        let err = fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap_err();
        assert!(err.contains("войдите заново"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        s.delete("mail.m1", oauth::REFRESH).unwrap();
        let err = fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap_err();
        assert!(err.contains("войдите заново") && hits.load(Ordering::SeqCst) == 1, "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_code_is_exchanged_with_the_verifier_and_no_client_secret() {
        let (url, _, bodies) = token_stub(vec![(200, r#"{"access_token":"a1","refresh_token":"r1","expires_in":100}"#)]).await;
        let tokens = exchange(&url, "cid", "the-code", "the-verifier").await.unwrap();
        assert_eq!((tokens.access.as_str(), tokens.refresh.as_deref()), ("a1", Some("r1")));
        let body = bodies.lock().unwrap()[0].clone();
        assert!(body.contains("grant_type=authorization_code") && body.contains("code=the-code") && body.contains("code_verifier=the-verifier") && body.contains("client_id=cid"), "{body}");
        assert!(!body.contains("client_secret"), "{body}");
    }

    #[tokio::test]
    async fn the_address_comes_from_the_yandex_profile_with_the_token() {
        let (url, _, _) = token_stub(vec![(200, r#"{"default_email":"Ivan@Yandex.ru","login":"ivan"}"#)]).await;
        assert_eq!(fetch_email(&url, "tok").await.unwrap(), "ivan@yandex.ru");
        let (bad, _, _) = token_stub(vec![(200, r#"{"login":"ivan"}"#)]).await;
        assert!(fetch_email(&bad, "tok").await.is_err());
    }

    /// Запрос к петле одной строкой; возвращает первую строку ответа.
    async fn get(port: u16, target: &str) -> String {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        socket.write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = socket.read_to_string(&mut out).await;
        out.lines().next().unwrap_or("").to_string()
    }

    #[tokio::test]
    async fn the_callback_refuses_a_wrong_state_and_a_wrong_path_and_takes_the_right_one() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(crate::openrouter_login::wait_for_code(false, vec![Some(listener)], "good-state", CALLBACK_PATH));
        assert!(get(port, "/yandex-mail?code=evil&state=bad-state").await.contains("400"), "чужой state");
        assert!(get(port, "/yandex-mail?code=evil").await.contains("400"), "без state");
        assert!(get(port, "/cb?code=evil&state=good-state").await.contains("404"), "не тот путь");
        assert!(!waiting.is_finished(), "чужие ответы вход не обрывают и код не отдают");
        assert!(get(port, "/yandex-mail?code=the-code&state=good-state").await.contains("200"));
        assert_eq!(waiting.await.unwrap().unwrap(), "the-code");
        // После этого порт закрыт.
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err(), "слушатель закрыт");
    }

    async fn app_in(name: &str) -> (Arc<App>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mail-oauth-{name}-{}", crate::store::uid("t")));
        std::fs::create_dir_all(&dir).unwrap();
        (crate::scheduler::tests::make_app(dir.clone()).await, dir)
    }

    fn ready_flow(app: &App, id: &str, at: i64) {
        let outcome = Arc::new(Mutex::new(Outcome::Ready(Tokens { access: "a".into(), refresh: Some("r".into()), expires_in: Some(3600) }, "me@yandex.ru".into(), at)));
        *app.mail.oauth.lock().unwrap() = Some(Flow { id: id.into(), outcome, task: tokio::spawn(async {}).abort_handle() });
    }

    #[tokio::test]
    async fn a_second_sign_in_takes_the_port_from_the_first() {
        let (app, dir) = app_in("twice").await;
        let (first, _) = begin(&app).await.expect("первый вход");
        let (second, _) = begin(&app).await.expect("второй вход сразу после первого");
        assert_ne!(first, second);
        assert!(status(&app, &first).is_none(), "прежний вход погашен");
        assert_eq!(status(&app, &second).unwrap()["status"], "waiting");
        drop(app.mail.oauth.lock().unwrap().take());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_finished_sign_in_nobody_completed_is_forgotten_after_fifteen_minutes() {
        let (app, dir) = app_in("stale").await;
        ready_flow(&app, "f1", crate::store::now());
        assert_eq!(status(&app, "f1").unwrap()["status"], "ready");
        ready_flow(&app, "f2", crate::store::now() - READY_TTL_MS - 1000);
        let state = status(&app, "f2").unwrap();
        assert_eq!(state["status"], "failed");
        assert!(app.mail.oauth.lock().unwrap().is_none(), "токены удалены из памяти");
        ready_flow(&app, "f3", crate::store::now() - READY_TTL_MS - 1000);
        assert!(take(&app, "f3").unwrap().is_err());
        assert!(take(&app, "f3").is_none());
        ready_flow(&app, "f4", crate::store::now());
        assert_eq!(take(&app, "f4").unwrap().unwrap().0, "me@yandex.ru");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_token_without_a_recorded_expiry_is_refreshed_when_it_can_be() {
        let (s, dir) = secrets("noexpiry");
        let (url, hits, _) = token_stub(vec![(200, r#"{"access_token":"new-a","expires_in":3600}"#)]).await;
        s.set("mail.m1", oauth::ACCESS, "old-a").unwrap();
        s.set("mail.m1", oauth::REFRESH, "r1").unwrap();
        assert_eq!(fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap(), "new-a");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // Обновить нечем: токен используем как есть.
        s.delete("mail.m1", oauth::REFRESH).unwrap();
        s.delete("mail.m1", oauth::EXPIRES).unwrap();
        assert_eq!(fresh_token(&s, "mail.m1", &url, "cid", false).await.unwrap(), "new-a");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_exclusive_port_rejects_a_reuseaddr_binder() {
        let listener = bind_port().await.expect("bind_port");
        use windows_sys::Win32::Networking::WinSock::*;
        let res = unsafe {
            let s = socket(AF_INET as i32, SOCK_STREAM, IPPROTO_TCP);
            assert!(s != INVALID_SOCKET);
            let opt: i32 = 1;
            setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &opt as *const i32 as *const u8, std::mem::size_of::<i32>() as i32);
            let mut addr: SOCKADDR_IN = std::mem::zeroed();
            addr.sin_family = AF_INET;
            addr.sin_port = htons(PORT);
            addr.sin_addr.S_un.S_addr = inet_addr(b"127.0.0.1\0".as_ptr());
            let r = bind(s, &addr as *const SOCKADDR_IN as *const SOCKADDR, std::mem::size_of::<SOCKADDR_IN>() as i32);
            closesocket(s);
            r
        };
        drop(listener);
        assert_ne!(res, 0, "a SO_REUSEADDR socket must not bind the exclusive port");
    }
}
