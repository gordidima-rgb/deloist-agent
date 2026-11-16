//! Секреты подключений: ключи и токены приложений из каталога.
//!
//! Лежат не в `state.json`, а в `data/secrets/connectors.bin`, зашифрованные DPAPI текущего
//! пользователя Windows: другой пользователь машины и копия файла на другом компьютере их не
//! прочитают. Внутри — JSON `{область: {имя: значение}}`; область — id MCP-сервера, и сервер
//! видит только свою. Значения наружу не отдаются: ни странице, ни журналу, ни модели.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

type Scopes = BTreeMap<String, BTreeMap<String, String>>;

/// Префикс имён, которые пишет только вход через OAuth.
pub const RESERVED_PREFIX: &str = "oauth.";

pub struct Secrets {
    path: PathBuf,
    data: Mutex<Scopes>,
    /// Файл есть, но не прочитан: запись закрыта, чтобы не затереть его.
    unavailable: AtomicBool,
}

/// Имя секрета: коротко и без символов, ломающих шаблон `{{secret:NAME}}`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

impl Secrets {
    /// Открыть хранилище в папке данных. Нечитаемый файл (чужой пользователь, порча) не роняет
    /// приложение: секретов просто нет, подключения попросят их заново.
    pub fn open(data_dir: &Path) -> Self {
        Self::open_with(data_dir, |path| std::fs::read(path))
    }

    /// То же с подменяемым чтением файла (тест имитирует отказ в доступе).
    /// Пусто начинаем только когда файла нет. Любая другая ошибка чтения и любой отказ расшифровки
    /// дают пустое хранилище с флагом «недоступно»: запись закрыта, файл на диске не трогаем
    /// (его мог зашифровать другой пользователь или потерянный ключ). Откладывает файл только
    /// человек (`reset_unavailable`).
    fn open_with(data_dir: &Path, read: impl Fn(&Path) -> std::io::Result<Vec<u8>>) -> Self {
        note_data_dir(data_dir);
        let path = data_dir.join("secrets").join("connectors.bin");
        let mut unavailable = false;
        let data = match read(&path) {
            Ok(blob) => match unprotect(&blob).and_then(|plain| serde_json::from_slice(&plain).map_err(|e| e.to_string())) {
                Ok(data) => data,
                Err(e) => {
                    // Открытый остаток старой версии вне Windows: затираем, а не откладываем.
                    #[cfg(not(windows))]
                    if is_plaintext_leftover(&blob) {
                        eprintln!("[secrets] открытый файл секретов не принят ({e}); затёрт: {}", shred_file(&path));
                        return Self { path, data: Mutex::new(Scopes::new()), unavailable: AtomicBool::new(false) };
                    }
                    // Windows: открытый JSON вместо DPAPI-блоба (остаток старой версии) отложен, как раньше.
                    #[cfg(windows)]
                    if blob.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
                        let aside = path.with_extension(format!("bin.bad-{}", crate::store::now()));
                        let moved = std::fs::rename(&path, &aside).is_ok();
                        eprintln!("[secrets] файл секретов не прочитан ({e}); отложен: {moved}");
                        return Self { path, data: Mutex::new(Scopes::new()), unavailable: AtomicBool::new(false) };
                    }
                    eprintln!("[secrets] файл секретов не прочитан ({e}); файл не трогаю, запись закрыта");
                    unavailable = true;
                    Scopes::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Scopes::new(),
            Err(e) => {
                eprintln!("[secrets] файл секретов не открылся ({}); файл не трогаю, запись закрыта", e.kind());
                unavailable = true;
                Scopes::new()
            }
        };
        Self { path, data: Mutex::new(data), unavailable: AtomicBool::new(unavailable) }
    }

    /// Файл не прочитался при запуске, запись закрыта.
    pub fn unavailable(&self) -> bool {
        self.unavailable.load(Ordering::SeqCst)
    }

    /// Выход из «недоступно» по решению человека: файл откладывается в `connectors.bin.bad-reset-N`
    /// (чистка такие не трогает), запись открывается. Возвращает, было ли что откладывать.
    pub fn reset_unavailable(&self) -> Result<bool, String> {
        let _data = self.data.lock().unwrap();
        if !self.unavailable() {
            return Ok(false);
        }
        let mut moved = false;
        if std::fs::symlink_metadata(&self.path).is_ok() {
            let aside = self.path.with_extension(format!("bin.bad-reset-{}", crate::store::now()));
            std::fs::rename(&self.path, &aside).map_err(|e| format!("не удалось отложить файл подключений: {e}"))?;
            moved = true;
        }
        self.unavailable.store(false, Ordering::SeqCst);
        Ok(moved)
    }

    pub fn get(&self, scope: &str, name: &str) -> Option<String> {
        self.data.lock().unwrap().get(scope)?.get(name).cloned()
    }

    pub fn names(&self, scope: &str) -> Vec<String> {
        self.data.lock().unwrap().get(scope).map(|m| m.keys().cloned().collect()).unwrap_or_default()
    }

    pub fn set(&self, scope: &str, name: &str, value: &str) -> Result<(), String> {
        if self.unavailable() {
            return Err("Ключи подключений сейчас недоступны: файл не прочитался, он цел. Запись закрыта".to_string());
        }
        if !valid_name(name) {
            return Err(format!("недопустимое имя секрета «{name}»"));
        }
        let mut data = self.data.lock().unwrap();
        data.entry(scope.to_string()).or_default().insert(name.to_string(), value.to_string());
        self.save(&data)
    }

    pub fn delete(&self, scope: &str, name: &str) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        let Some(map) = data.get_mut(scope) else { return Ok(()) };
        if map.remove(name).is_none() {
            return Ok(());
        }
        if map.is_empty() {
            data.remove(scope);
        }
        self.save(&data)
    }

    pub fn delete_scope(&self, scope: &str) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        if data.remove(scope).is_none() {
            return Ok(());
        }
        self.save(&data)
    }

    /// Записать целиком через временный файл: оборванная запись не теряет прежние секреты.
    fn save(&self, data: &Scopes) -> Result<(), String> {
        if self.unavailable() {
            return Err("Ключи подключений сейчас недоступны: файл не прочитался, он цел. Запись закрыта".to_string());
        }
        let plain = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        let blob = protect(&plain)?;
        let dir = self.path.parent().ok_or("нет папки секретов")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("папка секретов: {e}"))?;
        let tmp = self.path.with_extension("bin.tmp");
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            // Вне Windows файл не шифруется системой (нет DPAPI): читать его может только владелец.
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp).map_err(|e| format!("запись секретов: {e}"))?;
            file.write_all(&blob).map_err(|e| format!("запись секретов: {e}"))?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("запись секретов: {e}"))
    }
}

/// Своя добавка к ключу DPAPI: чужая программа того же пользователя, вызвав DPAPI без неё,
/// файл не расшифрует.
const ENTROPY: &[u8] = b"deloist-connectors-v1";

// ---------------------------------------------------------------------------
// `secrets.json`: ключи провайдеров, токен аккаунта и мессенджеров
// ---------------------------------------------------------------------------

/// Начало зашифрованного `secrets.json`. Имя файла прежнее: на него уже настроены все запреты
/// для инструментов бота (`reach`, `sandbox`, `keys`). Открытый JSON начинается с `{`.
pub const SEALED_MAGIC: &[u8] = b"DLSEC1\n";
/// Добавка DPAPI для `secrets.json`: своя, чтобы блоб одного файла не подходил к другому.
const FILE_ENTROPY: &[u8] = b"deloist-secrets-v1";

/// Что лежит в `secrets.json`.
pub enum FileContent {
    /// Расшифрованный JSON.
    Sealed(Vec<u8>),
    /// Старый открытый JSON: его надо перешифровать.
    Plain(Vec<u8>),
}

/// Зашифровать содержимое `secrets.json` DPAPI текущего пользователя.
pub fn seal_file(plain: &[u8]) -> Result<Vec<u8>, String> {
    let blob = dpapi_with(plain, FILE_ENTROPY, true)?;
    let mut out = Vec::with_capacity(SEALED_MAGIC.len() + blob.len());
    out.extend_from_slice(SEALED_MAGIC);
    out.extend_from_slice(&blob);
    Ok(out)
}

/// Прочитать `secrets.json`: зашифрованный или старый открытый. Ошибка — файл не наш или
/// зашифрован другим пользователем Windows. Текст ошибки секретов не содержит.
pub fn open_file(bytes: &[u8]) -> Result<FileContent, String> {
    if let Some(blob) = bytes.strip_prefix(SEALED_MAGIC) {
        return dpapi_with(blob, FILE_ENTROPY, false).map(FileContent::Sealed);
    }
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        return Ok(FileContent::Plain(text.to_vec()));
    }
    Err("файл секретов не распознан".to_string())
}

#[cfg(windows)]
fn dpapi(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    dpapi_with(input, ENTROPY, encrypt)
}

#[cfg(windows)]
fn dpapi_with(input: &[u8], entropy: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    let data_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: entropy.len() as u32, pbData: entropy.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // Буферы живут до конца вызова; выходной буфер DPAPI освобождаем LocalFree.
    let done = unsafe {
        if encrypt {
            CryptProtectData(&data_in, std::ptr::null(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&data_in, std::ptr::null_mut(), &entropy, std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if done == 0 || out.pbData.is_null() {
        let code = std::io::Error::last_os_error();
        return Err(if encrypt { format!("DPAPI не зашифровал: {code}") } else { format!("DPAPI не расшифровал: {code}") });
    }
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        LocalFree(out.pbData as _);
    }
    Ok(bytes)
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(plain, true)
}

#[cfg(windows)]
fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    dpapi(blob, false)
}

// Вне Windows DPAPI нет: XChaCha20-Poly1305 с мастер-ключом из файла или из окружения облака
// (`secrets_seal.rs`). Нет ключа или блоб чужой — ошибка, открытый текст не пишем и не отдаём.
#[cfg(not(windows))]
#[path = "secrets_seal.rs"]
pub mod seal;

#[cfg(not(windows))]
fn dpapi_with(input: &[u8], entropy: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    if encrypt {
        seal::encrypt(input, entropy)
    } else {
        seal::decrypt(input, entropy)
    }
}

#[cfg(not(windows))]
fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    seal::encrypt(plain, ENTROPY)
}

#[cfg(not(windows))]
fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    seal::decrypt(blob, ENTROPY)
}

/// Запомнить папку данных: без неё нельзя понять, что пропавший файл ключа — потеря, а не первый
/// запуск (`secrets_seal::load_or_create_key_file`). Вне Windows; в модульных тестах не нужна.
#[cfg(not(windows))]
pub fn note_data_dir(dir: &Path) {
    #[cfg(not(test))]
    seal::note_data_dir(dir);
    #[cfg(test)]
    let _ = dir;
}

#[cfg(windows)]
pub fn note_data_dir(_dir: &Path) {}

/// Активный файл мастер-ключа (для запретов инструментам бота). На Windows ключ даёт DPAPI.
#[cfg(not(windows))]
pub fn key_file_path() -> Option<PathBuf> {
    seal::key_file_path().ok()
}

#[cfg(windows)]
pub fn key_file_path() -> Option<PathBuf> {
    None
}

/// Открытые остатки от прежних версий без шифрования (вне Windows): `{…}` или `DLSEC1` с открытым
/// JSON внутри. Зашифрованный блок начинается с `DLX1`.
#[cfg(not(windows))]
pub fn is_plaintext_leftover(bytes: &[u8]) -> bool {
    if let Some(rest) = bytes.strip_prefix(SEALED_MAGIC) {
        return !rest.starts_with(b"DLX1");
    }
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

/// Затереть файл нулями и удалить: открытые секреты не остаются в отложенных копиях.
#[cfg(not(windows))]
pub fn shred_file(path: &Path) -> bool {
    use std::io::Write;
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(path) {
            let zeros = vec![0u8; meta.len() as usize];
            let _ = file.write_all(&zeros);
            let _ = file.sync_all();
        }
    }
    std::fs::remove_file(path).is_ok()
}

/// Первым делом в `main`: в облачном режиме забирает `CREW_SECRETS_KEY` из окружения.
#[cfg(not(windows))]
pub fn init_at_startup(args: &[String]) -> Result<(), String> {
    seal::init_cloud_from_env(args)
}

/// На Windows ключ даёт DPAPI, подготовки не нужно.
#[cfg(windows)]
pub fn init_at_startup(_args: &[String]) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Шаблоны `{{secret:NAME}}`
// ---------------------------------------------------------------------------

const OPEN: &str = "{{secret:";
const CLOSE: &str = "}}";

/// Имена секретов в шаблоне, по порядку и без повторов.
pub fn names_in(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else { break };
        let name = after[..end].trim().to_string();
        if !out.contains(&name) {
            out.push(name);
        }
        rest = &after[end + CLOSE.len()..];
    }
    out
}

pub fn has_reference(text: &str) -> bool {
    text.contains(OPEN)
}

/// Подставить секреты. Ошибка называет недостающий секрет, но не значения.
pub fn substitute(template: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            return Err("незакрытая ссылка {{secret:…}}".into());
        };
        let name = after[..end].trim();
        if !valid_name(name) {
            return Err(format!("недопустимое имя секрета «{name}»"));
        }
        match lookup(name) {
            Some(value) if !value.is_empty() => out.push_str(&value),
            _ => return Err(format!("нет секрета {name}")),
        }
        rest = &after[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Как показать шаблон: ссылки на секреты остаются, любой вписанный текст — точками
/// (в нём мог оказаться ключ: `Bearer sk-…` руками).
pub fn masked(template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    let literal = |text: &str, out: &mut String| {
        if !text.is_empty() {
            out.push_str("•••");
        }
    };
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else { break };
        literal(&rest[..start], &mut out);
        out.push_str(&rest[start..start + OPEN.len() + end + CLOSE.len()]);
        rest = &after[end + CLOSE.len()..];
    }
    literal(rest, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crew-secrets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn dpapi_round_trip_keeps_values_off_the_disk() {
        let dir = temp("round");
        let store = Secrets::open(&dir);
        store.set("srv", "token", "sk-very-secret-42").unwrap();
        store.set("srv", "other", "x").unwrap();
        store.set("srv2", "token", "y").unwrap();

        let raw = std::fs::read(dir.join("secrets").join("connectors.bin")).unwrap();
        assert!(!raw.windows(17).any(|w| w == b"sk-very-secret-42"), "значение лежит на диске открыто");

        let again = Secrets::open(&dir);
        assert_eq!(again.get("srv", "token").as_deref(), Some("sk-very-secret-42"));
        assert_eq!(again.names("srv"), vec!["other".to_string(), "token".to_string()]);
        again.delete("srv", "other").unwrap();
        again.delete_scope("srv2").unwrap();
        let third = Secrets::open(&dir);
        assert_eq!(third.names("srv"), vec!["token".to_string()]);
        assert!(third.get("srv2", "token").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn secrets_json_is_sealed_and_never_plain() {
        let sealed = seal_file(br#"{"apiKey":"sk-plain-marker-77"}"#).unwrap();
        assert!(sealed.starts_with(SEALED_MAGIC));
        assert!(!sealed.windows(17).any(|w| w == b"sk-plain-marker-7"), "значение в блобе открыто");
        match open_file(&sealed).unwrap() {
            FileContent::Sealed(plain) => assert_eq!(plain, br#"{"apiKey":"sk-plain-marker-77"}"#),
            FileContent::Plain(_) => panic!("ждали расшифрованный файл"),
        }
        // Заголовок «зашифровано» над открытым текстом не принимается.
        let mut fake = SEALED_MAGIC.to_vec();
        fake.extend_from_slice(br#"{"apiKey":"x"}"#);
        assert!(open_file(&fake).is_err());
        // Блоб подключений не подходит к secrets.json: у назначений разные ключи.
        let other = protect(b"{}").unwrap();
        let mut cross = SEALED_MAGIC.to_vec();
        cross.extend_from_slice(&other);
        assert!(open_file(&cross).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn a_plain_connectors_file_is_shredded_on_unix() {
        let dir = temp("foreign-unix");
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        std::fs::write(dir.join("secrets").join("connectors.bin"), b"{\"srv\":{\"token\":\"plain\"}}").unwrap();
        let store = Secrets::open(&dir);
        assert!(store.get("srv", "token").is_none());
        let left: Vec<_> = std::fs::read_dir(dir.join("secrets")).unwrap().flatten().collect();
        assert!(left.is_empty(), "открытый файл не остаётся даже отложенным: {left:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_foreign_blob_is_not_read() {
        let dir = temp("foreign");
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        std::fs::write(dir.join("secrets").join("connectors.bin"), b"{\"srv\":{\"token\":\"plain\"}}").unwrap();
        let store = Secrets::open(&dir);
        assert!(store.get("srv", "token").is_none(), "открытый JSON вместо DPAPI не принимаем");
        let aside: Vec<_> = std::fs::read_dir(dir.join("secrets")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(aside.iter().any(|n| n.starts_with("connectors.bin.bad-")), "нечитаемый файл отложен, а не стёрт: {aside:?}");
        store.set("srv", "token", "new").unwrap();
        let aside_count = std::fs::read_dir(dir.join("secrets")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("connectors.bin.bad-")).count();
        assert_eq!(aside_count, 1, "запись нового не трогает отложенный файл");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_undecryptable_connectors_file_is_untouched_and_blocks_saves() {
        let dir = temp("undecryptable");
        let file = dir.join("secrets").join("connectors.bin");
        std::fs::create_dir_all(dir.join("secrets")).unwrap();
        // Блоб не нашего ключа и не открытый JSON: расшифровка не пройдёт.
        let junk = b"DLX1-not-our-key-not-our-key-not-our-key-not-our-key".to_vec();
        std::fs::write(&file, &junk).unwrap();
        let store = Secrets::open(&dir);
        assert!(store.unavailable());
        assert!(store.set("srv", "token", "new").is_err(), "запись отказана");
        assert_eq!(std::fs::read(&file).unwrap(), junk, "файл без изменений");
        assert_eq!(std::fs::read_dir(dir.join("secrets")).unwrap().count(), 1, "ничего не отложено");
        // Человек выбирает «начать с пустыми ключами»: копия остаётся, запись открыта.
        assert!(store.reset_unavailable().unwrap());
        assert!(!store.unavailable());
        let names: Vec<String> = std::fs::read_dir(dir.join("secrets")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].starts_with("connectors.bin.bad-reset-"), "{names:?}");
        assert_eq!(std::fs::read(dir.join("secrets").join(&names[0])).unwrap(), junk);
        store.set("srv", "token", "new").unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_failure_other_than_not_found_never_leads_to_an_overwrite() {
        let dir = temp("eacces");
        // Настоящий файл с настоящими секретами; чтение «отказывает в доступе».
        let good = Secrets::open(&dir);
        good.set("srv", "token", "sk-keep-me").unwrap();
        let file = dir.join("secrets").join("connectors.bin");
        let before = std::fs::read(&file).unwrap();
        let denied = Secrets::open_with(&dir, |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
        assert!(denied.unavailable());
        assert!(denied.get("srv", "token").is_none());
        assert!(denied.set("srv", "token", "overwrite").is_err());
        assert!(denied.delete_scope("srv").is_ok(), "пустое хранилище нечего удалять");
        assert_eq!(std::fs::read(&file).unwrap(), before, "connectors.bin не перезаписан");
        // Нет файла — это просто пусто.
        let fresh = Secrets::open_with(&dir, |_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)));
        assert!(!fresh.unavailable());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn substitution_names_the_missing_secret_only() {
        let lookup = |name: &str| (name == "token").then(|| "abc".to_string());
        assert_eq!(substitute("Bearer {{secret:token}}", lookup).unwrap(), "Bearer abc");
        assert_eq!(substitute("plain", lookup).unwrap(), "plain");
        assert_eq!(substitute("{{secret:token}}-{{secret: token }}", lookup).unwrap(), "abc-abc");
        let error = substitute("{{secret:other}}", lookup).unwrap_err();
        assert!(error.contains("other") && !error.contains("abc"), "{error}");
        assert!(substitute("{{secret:token", lookup).is_err());
        assert!(substitute("{{secret:a b}}", lookup).is_err());
        assert_eq!(names_in("{{secret:a}} {{secret:b}} {{secret:a}}"), vec!["a", "b"]);
    }

    #[test]
    fn masking_hides_literal_values() {
        assert_eq!(masked("Bearer {{secret:t}}"), "•••{{secret:t}}");
        assert_eq!(masked("{{secret:a}}:{{secret:b}}"), "{{secret:a}}•••{{secret:b}}");
        assert_eq!(masked("{{secret:t}}"), "{{secret:t}}");
        assert_eq!(masked("sk-typed-by-hand"), "•••");
        assert_eq!(masked(""), "");
    }

    #[test]
    fn names_are_short_and_plain() {
        assert!(valid_name("oauth.access_token"));
        assert!(valid_name("API_KEY-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("a}}b"));
        assert!(!valid_name(&"x".repeat(65)));
    }
}
