//! Шифрование секретов вне Windows (там работает DPAPI, см. `secrets.rs`).
//!
//! XChaCha20-Poly1305. Блоб: `DLX1` + 24 байта nonce + шифртекст с тегом. Ключ назначения —
//! SHA-256(мастер-ключ || `deloist-secrets-v1` || entropy); entropy идёт и как AAD, поэтому блоб
//! одного назначения не открывается как другой. Любой сбой — `Err`, вход назад не отдаётся.
//!
//! Мастер-ключ: в облаке (`--server` и `CREW_CLOUD=1`) обязательная переменная
//! `CREW_SECRETS_KEY` (64 hex), которую `main` убирает из окружения до старта потоков; иначе
//! файл `$XDG_CONFIG_HOME/deloist/secrets.key` (или `$HOME/.config/deloist/secrets.key`), права 0600.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

const MAGIC: &[u8] = b"DLX1";
const DOMAIN: &[u8] = b"deloist-secrets-v1";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Имя переменной с мастер-ключом облака.
pub const KEY_ENV: &str = "CREW_SECRETS_KEY";
/// Путь к файлу ключа (для тестов и нестандартных установок).
pub const KEY_FILE_ENV: &str = "CREW_SECRETS_KEY_FILE";
/// Имя файла ключа.
pub const KEY_FILE_NAME: &str = "secrets.key";

pub type Master = [u8; 32];

static MASTER: OnceLock<Result<Master, String>> = OnceLock::new();

fn derive(master: &Master, entropy: &[u8]) -> Key {
    let mut hasher = Sha256::new();
    hasher.update(master);
    hasher.update(DOMAIN);
    hasher.update(entropy);
    let digest: [u8; 32] = hasher.finalize().into();
    Key::from(digest)
}

pub fn encrypt_with(master: &Master, plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new(&derive(master, entropy));
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let sealed = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: plain, aad: entropy })
        .map_err(|_| "не удалось зашифровать секреты".to_string())?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

pub fn decrypt_with(master: &Master, blob: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    let rest = blob.strip_prefix(MAGIC).ok_or("секреты не зашифрованы нашим ключом")?;
    if rest.len() < NONCE_LEN + TAG_LEN {
        return Err("блок секретов обрезан".into());
    }
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(nonce);
    let cipher = XChaCha20Poly1305::new(&derive(master, entropy));
    cipher
        .decrypt(&XNonce::from(nonce_bytes), Payload { msg: sealed, aad: entropy })
        .map_err(|_| "секреты не расшифрованы: другой ключ или файл испорчен".to_string())
}

/// Зашифровать мастер-ключом процесса.
pub fn encrypt(plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    encrypt_with(master()?, plain, entropy)
}

/// Расшифровать мастер-ключом процесса.
pub fn decrypt(blob: &[u8], entropy: &[u8]) -> Result<Vec<u8>, String> {
    decrypt_with(master()?, blob, entropy)
}

fn master() -> Result<&'static Master, String> {
    MASTER.get_or_init(resolve_local).as_ref().map_err(|e| e.clone())
}

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Папка данных, где лежат зашифрованные файлы; запоминается один раз при открытии хранилищ.
pub fn note_data_dir(dir: &Path) {
    let _ = DATA_DIR.set(dir.to_path_buf());
}

/// Есть ли в папке данных файлы, зашифрованные нашим ключом: `secrets.json` с `DLSEC1` + `DLX1`
/// или непустой `secrets/connectors.bin` с `DLX1`.
pub fn sealed_files_exist(data_dir: &Path) -> bool {
    let sealed_json = std::fs::read(data_dir.join("secrets.json")).is_ok_and(|b| b.strip_prefix(crate::secrets::SEALED_MAGIC).is_some_and(|rest| rest.starts_with(MAGIC)));
    let sealed_bin = std::fs::read(data_dir.join("secrets").join("connectors.bin")).is_ok_and(|b| b.starts_with(MAGIC));
    sealed_json || sealed_bin
}

fn resolve_local() -> Result<Master, String> {
    load_or_create_key_file(&key_file_path()?, DATA_DIR.get().map(PathBuf::as_path))
}

/// Читает мастер-ключ облака из переменной и сразу убирает её из окружения.
pub fn take_hex_key_env(name: &str) -> Result<Master, String> {
    let value = std::env::var(name);
    std::env::remove_var(name);
    let value = value.map_err(|_| format!("{name} не задан: в облачном режиме нужен ключ шифрования секретов (64 hex-символа)"))?;
    parse_hex_key(value.trim()).ok_or_else(|| format!("{name} неверный: нужно ровно 64 hex-символа (32 байта)"))
}

fn parse_hex_key(text: &str) -> Option<Master> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Закрывает чтение памяти и окружения процесса другими процессами того же пользователя:
/// `remove_var` не чистит `/proc/<pid>/environ`, а бот с тем же uid иначе прочёл бы оттуда ключ
/// (или из `/proc/<pid>/mem`). Дети после `exec` снова обычные, их окружение своё.
#[cfg(target_os = "linux")]
fn set_undumpable() -> Result<(), String> {
    // SAFETY: prctl с константами, без указателей.
    let rc = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("prctl(PR_SET_DUMPABLE) не удался: {}", std::io::Error::last_os_error()))
    }
}

#[cfg(not(target_os = "linux"))]
fn set_undumpable() -> Result<(), String> {
    Ok(())
}

/// Вызывается первым делом в `main`, до рантайма, потоков и дочерних процессов.
/// 1) На Linux всегда закрывает дамп процесса (в облаке отказ — ошибка запуска).
/// 2) В любом режиме убирает `CREW_SECRETS_KEY` из окружения: команды ботов и агенты наследуют
///    окружение процесса.
/// 3) В облачном режиме (`--server` и `CREW_CLOUD=1`) ключ обязателен и становится мастер-ключом.
///    Задан без облачного режима — отказ: молча уйти на файл ключа нельзя.
pub fn init_cloud_from_env(args: &[String]) -> Result<(), String> {
    let cloud = is_cloud(args, std::env::var("CREW_CLOUD").ok().as_deref());
    let dumpable = set_undumpable();
    if cloud {
        dumpable?;
        let key = take_hex_key_env(KEY_ENV)?;
        let _ = MASTER.set(Ok(key));
        return Ok(());
    }
    let was_set = std::env::var_os(KEY_ENV).is_some();
    std::env::remove_var(KEY_ENV);
    if was_set {
        return Err(format!("{KEY_ENV} set without CREW_CLOUD=1: ключ шифрования задан вне облачного режима, запуск отменён"));
    }
    Ok(())
}

pub fn is_cloud(args: &[String], cloud_env: Option<&str>) -> bool {
    cloud_env == Some("1") && args.iter().any(|a| a == "--server")
}

pub fn key_file_path() -> Result<PathBuf, String> {
    // Свой путь к ключу — только для тестов (отладочная сборка): в выпуске его окружением не подсунуть.
    if cfg!(debug_assertions) {
        if let Some(path) = std::env::var_os(KEY_FILE_ENV).filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(path));
        }
    }
    // Модульные тесты не трогают настоящий каталог настроек пользователя.
    if cfg!(test) {
        return Ok(std::env::temp_dir().join("deloist-unit-test-keys").join(KEY_FILE_NAME));
    }
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let home = std::env::var_os("HOME").filter(|v| !v.is_empty()).ok_or("нет HOME и XDG_CONFIG_HOME: негде хранить ключ шифрования секретов")?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(base.join("deloist").join(KEY_FILE_NAME))
}

#[cfg(unix)]
pub fn load_or_create_key_file(path: &Path, data_dir: Option<&Path>) -> Result<Master, String> {
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

    // SAFETY: geteuid без аргументов.
    let euid = unsafe { libc::geteuid() };
    if let Some(dir) = path.parent() {
        match std::fs::metadata(dir) {
            Ok(meta) => {
                // Существующую папку не правим молча: чужая или открытая на запись — отказ.
                if !meta.is_dir() {
                    return Err(format!("{} не папка: ключ секретов там не хранится", dir.display()));
                }
                if meta.uid() != euid {
                    return Err(format!("папка ключа секретов {} принадлежит другому пользователю", dir.display()));
                }
                if meta.mode() & 0o022 != 0 {
                    return Err(format!("папка ключа секретов {} доступна другим на запись: нужны права 0700", dir.display()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("папка ключа секретов: {e}"))?;
            }
            Err(e) => return Err(format!("папка ключа секретов: {e}")),
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => return Err("ключ секретов — символическая ссылка: отказ".into()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Ключа нет, а зашифрованные им файлы есть: это потеря ключа, а не первый запуск.
            // Новый ключ сделал бы их нечитаемыми навсегда, поэтому отказ, файлы не трогаем.
            if let Some(dir) = data_dir {
                if sealed_files_exist(dir) {
                    return Err("the secrets key file is missing; existing secrets cannot be read (файл ключа секретов пропал, прежние секреты не прочитать; новый ключ не создан)".into());
                }
            }
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            // Пишем во временный файл и вешаем жёсткой ссылкой: читатель не увидит недописанный ключ,
            // а два процесса не затрут ключ друг друга.
            let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
            let _ = std::fs::remove_file(&tmp);
            {
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(&tmp).map_err(|e| format!("запись ключа секретов: {e}"))?;
                file.write_all(&key).map_err(|e| format!("запись ключа секретов: {e}"))?;
            }
            let linked = std::fs::hard_link(&tmp, path);
            let _ = std::fs::remove_file(&tmp);
            if let Err(e) = linked {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(format!("запись ключа секретов: {e}"));
                }
            }
        }
        Err(e) => return Err(format!("ключ секретов: {e}")),
    }
    // Проверки и чтение идут по одному дескриптору, без гонки между ними.
    let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path).map_err(|e| format!("ключ секретов не открыт: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("ключ секретов: {e}"))?;
    if !meta.is_file() {
        return Err("ключ секретов: это не обычный файл".into());
    }
    if meta.uid() != euid {
        return Err("файл ключа секретов принадлежит другому пользователю".into());
    }
    if meta.mode() & 0o077 != 0 {
        return Err(format!("файл ключа секретов {} доступен другим пользователям: нужны права 0600", path.display()));
    }
    if meta.len() != 32 {
        return Err("файл ключа секретов испорчен: нужно ровно 32 байта".into());
    }
    let mut key = [0u8; 32];
    file.read_exact(&mut key).map_err(|e| format!("ключ секретов не прочитан: {e}"))?;
    Ok(key)
}

#[cfg(not(unix))]
pub fn load_or_create_key_file(_path: &Path, _data_dir: Option<&Path>) -> Result<Master, String> {
    Err("на этой системе нет безопасного хранилища ключа секретов".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENT: &[u8] = b"deloist-connectors-v1";

    #[test]
    fn round_trip_and_blob_shape() {
        let key = [7u8; 32];
        let blob = encrypt_with(&key, b"sk-secret-value", ENT).unwrap();
        assert!(blob.starts_with(b"DLX1"));
        assert!(!blob.windows(15).any(|w| w == b"sk-secret-value"));
        assert_eq!(decrypt_with(&key, &blob, ENT).unwrap(), b"sk-secret-value");
        let again = encrypt_with(&key, b"sk-secret-value", ENT).unwrap();
        assert_ne!(blob, again, "nonce случайный");
    }

    #[test]
    fn tampering_wrong_key_wrong_purpose_and_plaintext_are_errors() {
        let key = [7u8; 32];
        let blob = encrypt_with(&key, b"value", ENT).unwrap();
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(decrypt_with(&key, &bad, ENT).is_err(), "байт {i}");
        }
        assert!(decrypt_with(&[8u8; 32], &blob, ENT).is_err());
        assert!(decrypt_with(&key, &blob, b"deloist-secrets-v1").is_err(), "другое назначение");
        assert!(decrypt_with(&key, b"{\"srv\":{\"token\":\"plain\"}}", ENT).is_err());
        assert!(decrypt_with(&key, b"DLX1", ENT).is_err());
        assert!(decrypt_with(&key, &blob[..blob.len() - 1], ENT).is_err());
        assert!(decrypt_with(&key, b"", ENT).is_err());
    }

    #[test]
    fn cloud_key_is_read_and_removed_from_the_environment() {
        let name = "CREW_TEST_SECRETS_KEY_ONCE";
        std::env::set_var(name, "ab".repeat(32));
        assert_eq!(take_hex_key_env(name).unwrap(), [0xab; 32]);
        assert!(std::env::var_os(name).is_none(), "ключ остался в окружении");
        assert!(take_hex_key_env(name).is_err(), "нет переменной — ошибка");
        std::env::set_var(name, "xyz");
        assert!(take_hex_key_env(name).is_err());
        assert!(std::env::var_os(name).is_none(), "неверный ключ тоже убирается");
        std::env::set_var(name, "zz".repeat(32));
        assert!(take_hex_key_env(name).is_err());
    }

    #[test]
    fn cloud_mode_needs_both_server_and_the_flag() {
        let server = vec!["--server".to_string()];
        assert!(is_cloud(&server, Some("1")));
        assert!(!is_cloud(&server, None));
        assert!(!is_cloud(&server, Some("0")));
        assert!(!is_cloud(&[], Some("1")));
    }

    #[cfg(unix)]
    #[test]
    fn key_file_is_created_private_and_a_loose_one_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("deloist-keyfile-{}", std::process::id())).join("nested");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
        let path = dir.join(KEY_FILE_NAME);
        let key = load_or_create_key_file(&path, None).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(load_or_create_key_file(&path, None).unwrap(), key, "тот же ключ при повторном чтении");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create_key_file(&path, None).is_err(), "0644 отвергнут");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, b"short").unwrap();
        assert!(load_or_create_key_file(&path, None).is_err(), "не 32 байта — отказ");
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_key_and_a_foreign_writable_directory_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("deloist-keyfile-hard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("keys");
        let real = dir.join("real.key");
        let key = load_or_create_key_file(&real, None).unwrap();
        let link = dir.join(KEY_FILE_NAME);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = load_or_create_key_file(&link, None).unwrap_err();
        assert!(err.contains("символическая"), "{err}");
        assert_eq!(load_or_create_key_file(&real, None).unwrap(), key);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let err = load_or_create_key_file(&real, None).unwrap_err();
        assert!(err.contains("на запись"), "{err}");
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o777, "папку молча не правим");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_key_with_existing_sealed_files_is_refused_and_no_key_is_created() {
        let root = std::env::temp_dir().join(format!("deloist-keyfile-lost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("data");
        std::fs::create_dir_all(data.join("secrets")).unwrap();
        let keys = root.join("keys").join(KEY_FILE_NAME);
        // Первый запуск без секретов: ключ создаётся.
        assert!(load_or_create_key_file(&keys, Some(&data)).is_ok());
        std::fs::remove_file(&keys).unwrap();
        for (name, content) in [("connectors.bin", b"DLX1xxxx".to_vec()), ("../secrets.json", [crate::secrets::SEALED_MAGIC, b"DLX1xxxx"].concat())] {
            let file = data.join("secrets").join(name);
            std::fs::write(&file, &content).unwrap();
            let err = load_or_create_key_file(&keys, Some(&data)).unwrap_err();
            assert!(err.contains("key file is missing"), "{err}");
            assert!(!keys.exists(), "новый ключ не создан");
            assert_eq!(std::fs::read(&file).unwrap(), content, "файл не тронут");
            std::fs::remove_file(&file).unwrap();
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
