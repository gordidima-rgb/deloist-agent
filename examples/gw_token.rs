//! Пропуск входа на облачный стенд, пока нет настоящего шлюза deloist.ru (этап 4).
//! Закрытый ключ живёт только у владельца на ПК; на сервер уходит открытая часть (`CREW_GATEWAY_PUBKEY`).
//!
//!   gw_token keygen <файл ключа>                     создать ключ и напечатать открытую часть
//!   gw_token pubkey <файл ключа>                     напечатать открытую часть
//!   gw_token url <файл ключа> <uid> <origin>         напечатать ссылку входа (живёт 30 с, одноразовая)
//!
//! Формат пропуска тот же, что проверяет `src/guard.rs`: `{uid, aud, iat, exp, nonce}` в base64url,
//! подпись Ed25519 над `"deloist-gw-v1\n" + payload`.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;

const DOMAIN: &[u8] = b"deloist-gw-v1\n";

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
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", path] => {
            if std::path::Path::new(path).exists() {
                eprintln!("файл ключа уже есть, не перезаписываю: {path}");
                std::process::exit(1);
            }
            let mut seed = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut seed);
            if let Some(dir) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(dir).expect("папка ключа");
            }
            std::fs::write(path, to_hex(&seed)).expect("запись ключа");
            println!("{}", to_hex(SigningKey::from_bytes(&seed).verifying_key().as_bytes()));
        }
        ["pubkey", path] => println!("{}", to_hex(load_key(path).verifying_key().as_bytes())),
        ["url", path, uid, origin] => {
            let key = load_key(path);
            let origin = origin.trim_end_matches('/').to_lowercase();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let mut nonce = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut nonce);
            let payload = serde_json::json!({ "uid": uid, "aud": origin, "iat": now, "exp": now + 30, "nonce": to_hex(&nonce) }).to_string();
            let mut signed = DOMAIN.to_vec();
            signed.extend_from_slice(payload.as_bytes());
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            let token = format!("{}.{}", b64.encode(&payload), b64.encode(key.sign(&signed).to_bytes()));
            println!("{origin}/auth/gateway?t={token}");
        }
        _ => {
            eprintln!("gw_token keygen <файл> | pubkey <файл> | url <файл> <uid> <origin>");
            std::process::exit(2);
        }
    }
}
