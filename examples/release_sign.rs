//! Инструмент выпуска: создаёт ключ и подписывает новую версию. Закрытый ключ живёт вне репозитория.
//!
//!   release_sign keygen <файл ключа>                 создать ключ и напечатать открытую часть
//!   release_sign sign <файл ключа> <exe> <версия> <адрес exe> <заметки> <папка>
//!                                                    записать latest.json и latest.json.sig

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Vec<u8> {
    let text = text.trim();
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("ключ должен быть в hex")).collect()
}

fn load_key(path: &str) -> SigningKey {
    let bytes: [u8; 32] = from_hex(&std::fs::read_to_string(path).expect("нет файла ключа")).try_into().expect("длина ключа");
    SigningKey::from_bytes(&bytes)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = args.get(1).expect("укажи файл ключа");
            // Перезапись ключа отрезала бы всех пользователей от обновлений.
            assert!(!std::path::Path::new(path).exists(), "ключ уже есть, перезаписывать нельзя: {path}");
            if let Some(dir) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(dir).expect("папка ключа");
            }
            let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
            std::fs::write(path, to_hex(&key.to_bytes())).expect("запись ключа");
            println!("{}", to_hex(key.verifying_key().as_bytes()));
        }
        Some("sign") => {
            let [_, key, exe, version, url, notes, out] = args.as_slice() else {
                panic!("sign <ключ> <exe> <версия> <адрес exe> <заметки> <папка>");
            };
            let bytes = std::fs::read(exe).expect("нет exe");
            // `@файл` — заметки лежат в файле (несколько строк, кириллица без пересказа через командную строку).
            let notes = match notes.strip_prefix('@') {
                Some(path) => std::fs::read_to_string(path).expect("нет файла заметок"),
                None => notes.clone(),
            };
            assert!(notes.len() <= 16 * 1024, "заметки длиннее 16 КБ: клиент читает описание до 64 КБ, но заметки должны быть короткими");
            let mut manifest = serde_json::json!({
                "version": version,
                "notes": notes,
                "url": url,
                "sha256": to_hex(&Sha256::digest(&bytes)),
                "size": bytes.len(),
            });
            // Загрузчик WebView2 с 0.3.4 внутри exe: поля `loader` в новых описаниях нет.
            // Deloist-Setup.exe и старые версии программы принимают описание и с ним, и без него.
            let text = serde_json::to_string_pretty(&manifest).expect("описание версии");
            let signature = load_key(key).sign(text.as_bytes());
            std::fs::write(format!("{out}/latest.json"), &text).expect("запись описания");
            std::fs::write(format!("{out}/latest.json.sig"), to_hex(&signature.to_bytes())).expect("запись подписи");
            println!("{text}");
        }
        _ => panic!("команды: keygen, sign"),
    }
}
