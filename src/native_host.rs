//! Хост Native Messaging для расширения Deloist в Chrome человека.
//!
//! Chrome запускает `deloist.exe` с аргументом `chrome-extension://<id>/` и говорит с ним через
//! stdin/stdout кадрами «4 байта длины (little-endian) + JSON». Хост ничего не решает: проверяет,
//! что его позвало наше расширение, находит живую копию программы по `runtime.json`, подключается
//! к её именованному каналу и гоняет кадры в обе стороны. Канал — не сетевой порт: его не видно
//! по сети, а без ключа из `runtime.json` программа его закрывает. Политика (сайты, методы CDP,
//! работа без человека) живёт в `user_chrome.rs`, в самой программе.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Имя хоста в манифесте и в реестре.
pub const HOST_NAME: &str = "com.deloist.bots";

/// Id расширения. Первый — dev-сборка из папки `extension/` (ключ в её `manifest.json`).
/// Id из Chrome Web Store, когда владелец его опубликует, добавляется сюда же.
pub const EXTENSION_IDS: &[&str] = &["aicphofdeimagdepgjfkookblhldmcja"];

/// Chrome не принимает от хоста сообщения больше 1 МиБ.
pub const TO_EXTENSION_MAX: usize = 1024 * 1024;
/// От расширения приходят снимки экрана; Chrome сам режет на 64 МиБ, нам хватает 32.
pub const FROM_EXTENSION_MAX: usize = 32 * 1024 * 1024;
/// Первый кадр хоста программе: ключ и origin, больше там ничего нет.
pub const HELLO_MAX: usize = 64 * 1024;

pub fn pipe_name(pid: u32) -> String {
    format!(r"\\.\pipe\deloist-user-chrome-{pid}")
}

pub fn allowed_origins() -> Vec<String> {
    EXTENSION_IDS.iter().map(|id| format!("chrome-extension://{id}/")).collect()
}

/// Позвало ли нас наше расширение. Сравнение точное: `chrome-extension://<id>/`.
pub fn origin_allowed(origin: &str) -> bool {
    allowed_origins().iter().any(|allowed| allowed == origin)
}

/// Origin, с которым Chrome запустил хост: первый аргумент вида `chrome-extension://…`.
/// На Windows Chrome добавляет ещё `--parent-window=<n>`, его пропускаем.
pub fn caller_origin(args: &[String]) -> Option<String> {
    args.iter().find(|arg| arg.starts_with("chrome-extension://")).cloned()
}

/// Режим хоста: явный ключ `--native-host` или запуск самим Chrome (аргумент с origin).
pub fn is_host_call(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--native-host") || caller_origin(args).is_some()
}

// ---------------------------------------------------------------------------
// Кадры
// ---------------------------------------------------------------------------

pub fn encode(payload: &[u8], max: usize) -> Result<Vec<u8>, String> {
    if payload.len() > max {
        return Err(format!("кадр {} байт больше предела {max}", payload.len()));
    }
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn encode_json(value: &Value, max: usize) -> Result<Vec<u8>, String> {
    encode(value.to_string().as_bytes(), max)
}

/// Один кадр. `Ok(None)` — поток закончился ровно между кадрами. Длина больше `max` — ошибка
/// до чтения тела: память под чужую длину не выделяем.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, max: usize) -> Result<Option<Vec<u8>>, String> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        let n = reader.read(&mut head[got..]).await.map_err(|e| format!("чтение кадра: {e}"))?;
        if n == 0 {
            return if got == 0 { Ok(None) } else { Err("поток оборвался посреди длины кадра".into()) };
        }
        got += n;
    }
    let len = u32::from_le_bytes(head) as usize;
    if len > max {
        return Err(format!("кадр {len} байт больше предела {max}"));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await.map_err(|e| format!("поток оборвался посреди кадра: {e}"))?;
    Ok(Some(body))
}

pub async fn read_json<R: AsyncRead + Unpin>(reader: &mut R, max: usize) -> Result<Option<Value>, String> {
    match read_frame(reader, max).await? {
        None => Ok(None),
        Some(body) => serde_json::from_slice(&body).map(Some).map_err(|e| format!("кадр не JSON: {e}")),
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, payload: &[u8], max: usize) -> Result<(), String> {
    let frame = encode(payload, max)?;
    writer.write_all(&frame).await.map_err(|e| format!("запись кадра: {e}"))?;
    writer.flush().await.map_err(|e| format!("запись кадра: {e}"))
}

// ---------------------------------------------------------------------------
// Сам хост
// ---------------------------------------------------------------------------

/// Ответ расширению, почему хост не работает, и выход. Расширение показывает текст в своём окне.
async fn refuse(code: &str, text: &str) -> i32 {
    let mut stdout = tokio::io::stdout();
    let message = json!({ "type": "error", "code": code, "text": text });
    let _ = write_frame(&mut stdout, message.to_string().as_bytes(), TO_EXTENSION_MAX).await;
    1
}

pub fn run(data_dir: &Path, args: &[String]) -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(_) => return 1,
    };
    runtime.block_on(host(data_dir, args))
}

async fn host(data_dir: &Path, args: &[String]) -> i32 {
    let Some(origin) = caller_origin(args) else {
        return refuse("origin", "Хост Deloist запускает только Chrome или Edge для расширения Deloist.").await;
    };
    if !origin_allowed(&origin) {
        return refuse("origin", "Это расширение не Deloist: хост ему не отвечает.").await;
    }
    let Some(published) = crate::runtime::read(data_dir) else {
        return refuse("app_not_running", "Программа Deloist не запущена: открой её, и расширение подключится само.").await;
    };
    let pipe = match open_pipe(published.pid).await {
        Ok(pipe) => pipe,
        Err(text) => return refuse("app_not_running", &text).await,
    };
    relay(pipe, &published.token, &origin).await
}

#[cfg(windows)]
async fn open_pipe(pid: u32) -> Result<tokio::net::windows::named_pipe::NamedPipeClient, String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let name = pipe_name(pid);
    for _ in 0..40 {
        match ClientOptions::new().open(&name) {
            Ok(client) => {
                // Ключ отдаём только самой программе: канал с этим именем мог занять кто-то другой.
                if server_pid(&client) != Some(pid) {
                    return Err("Канал Deloist открыт не программой Deloist: хост ему ключ не отдаёт.".into());
                }
                return Ok(client);
            }
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(_) => return Err("Программа Deloist запущена, но не принимает расширение: обнови её до 0.3.1 или перезапусти.".into()),
        }
    }
    Err("Программа Deloist занята и не приняла расширение: попробуй ещё раз.".into())
}

#[cfg(windows)]
fn server_pid(client: &tokio::net::windows::named_pipe::NamedPipeClient) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    // Дескриптор живёт, пока жив `client`; функция только пишет число по указателю.
    let ok = unsafe { GetNamedPipeServerProcessId(client.as_raw_handle() as _, &mut pid) };
    (ok != 0).then_some(pid)
}

#[cfg(not(windows))]
async fn open_pipe(_pid: u32) -> Result<tokio::io::DuplexStream, String> {
    Err("«Мой Chrome» пока работает только в Windows.".into())
}

async fn relay<P: AsyncRead + AsyncWrite + Unpin>(pipe: P, token: &str, origin: &str) -> i32 {
    let (mut pipe_read, mut pipe_write) = tokio::io::split(pipe);
    let hello = json!({ "type": "host", "token": token, "origin": origin, "pid": std::process::id() });
    if write_frame(&mut pipe_write, hello.to_string().as_bytes(), HELLO_MAX).await.is_err() {
        return refuse("app_not_running", "Программа Deloist закрыла канал.").await;
    }
    let up = async {
        let mut stdin = tokio::io::stdin();
        loop {
            match read_frame(&mut stdin, FROM_EXTENSION_MAX).await {
                Ok(Some(frame)) => {
                    if write_frame(&mut pipe_write, &frame, FROM_EXTENSION_MAX).await.is_err() {
                        return;
                    }
                }
                // Chrome закрыл stdin: расширение отключилось или браузер закрыт.
                Ok(None) | Err(_) => return,
            }
        }
    };
    let down = async {
        let mut stdout = tokio::io::stdout();
        loop {
            match read_frame(&mut pipe_read, TO_EXTENSION_MAX).await {
                Ok(Some(frame)) => {
                    if write_frame(&mut stdout, &frame, TO_EXTENSION_MAX).await.is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => return,
            }
        }
    };
    tokio::select! {
        _ = up => {},
        _ = down => {},
    }
    0
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 для парного соединения расширение ↔ программа
// ---------------------------------------------------------------------------

/// HMAC-SHA256 (реализация поверх sha2, т.к. crate hmac не подключён).
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut key = key.to_vec();
    if key.len() > 64 {
        let h = Sha256::digest(&key);
        key = h.to_vec();
    }
    key.resize(64, 0);
    let mut ipad = vec![0x36u8; 64];
    let mut opad = vec![0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(msg);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    let res = outer.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&res);
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_rfc4231_case1() {
        let key = vec![0x0bu8; 20];
        let msg = b"Hi There";
        let mac = hmac_sha256(&key, msg);
        let got: String = mac.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(got, "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    }

    #[test]
    fn junk_hex_rejected_without_panic() {
        assert!(hex_decode("zz").is_none());
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("").is_some());
        assert!(hex_decode("0g").is_none());
    }

    #[tokio::test]
    async fn frames_round_trip_and_end_cleanly() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, br#"{"type":"hello"}"#, TO_EXTENSION_MAX).await.unwrap();
        write_frame(&mut buffer, "{\"t\":\"Привет\"}".as_bytes(), TO_EXTENSION_MAX).await.unwrap();
        assert_eq!(&buffer[..4], &16u32.to_le_bytes(), "длина little-endian, как у Chrome");
        let mut reader = &buffer[..];
        assert_eq!(read_json(&mut reader, 1024).await.unwrap().unwrap()["type"], "hello");
        assert_eq!(read_json(&mut reader, 1024).await.unwrap().unwrap()["t"], "Привет");
        assert!(read_frame(&mut reader, 1024).await.unwrap().is_none(), "конец потока между кадрами");
    }

    #[tokio::test]
    async fn oversized_and_torn_frames_are_refused() {
        // Чужая длина больше предела: отказ до чтения тела.
        let mut huge = Vec::new();
        huge.extend_from_slice(&(TO_EXTENSION_MAX as u32 + 1).to_le_bytes());
        let mut reader = &huge[..];
        assert!(read_frame(&mut reader, TO_EXTENSION_MAX).await.unwrap_err().contains("больше предела"));
        assert!(encode(&vec![b' '; TO_EXTENSION_MAX + 1], TO_EXTENSION_MAX).is_err(), "к расширению не больше 1 МиБ");
        assert!(encode(&vec![b' '; TO_EXTENSION_MAX + 1], FROM_EXTENSION_MAX).is_ok(), "от расширения можно больше");
        // Оборванная длина и оборванное тело.
        let mut reader: &[u8] = &[5, 0];
        assert!(read_frame(&mut reader, 1024).await.is_err());
        let mut reader: &[u8] = &[5, 0, 0, 0, b'{'];
        assert!(read_frame(&mut reader, 1024).await.is_err());
        let mut reader: &[u8] = &[2, 0, 0, 0, b'n', b'o'];
        assert!(read_json(&mut reader, 1024).await.unwrap_err().contains("не JSON"));
    }

    #[test]
    fn only_our_extension_is_a_caller() {
        let ours = format!("chrome-extension://{}/", EXTENSION_IDS[0]);
        assert!(origin_allowed(&ours));
        for other in [
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "chrome-extension://aicphofdeimagdepgjfkookblhldmcja",
            "chrome-extension://aicphofdeimagdepgjfkookblhldmcja/evil",
            "https://aicphofdeimagdepgjfkookblhldmcja/",
            "",
        ] {
            assert!(!origin_allowed(other), "{other}");
        }
        let args = vec![ours.clone(), "--parent-window=0".to_string()];
        assert_eq!(caller_origin(&args).as_deref(), Some(ours.as_str()));
        assert!(is_host_call(&args));
        assert!(is_host_call(&["--native-host".to_string()]));
        assert!(!is_host_call(&["--server".to_string()]));
    }
}
