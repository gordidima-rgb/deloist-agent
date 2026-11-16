//! Заглушка Bitwarden CLI (`bw`) для тестов менеджера паролей: только команды, которые использует программа.
//! Данные читаются из фикстуры `FAKE_VAULT_FIXTURE`. Каждый запуск дописывает строку аргументов в
//! `FAKE_VAULT_LOG`, чтобы тест видел: мастер-пароль и ключ сессии в аргументы не попадали.

use std::io::Write;

use serde_json::{json, Value};

const SESSION: &str = "fake-session-123";
const PASSWORD: &str = "correct horse";

fn log(args: &[String]) {
    let Ok(path) = std::env::var("FAKE_VAULT_LOG") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{}", args.join(" "));
    }
}

fn fixture() -> Value {
    let path = std::env::var("FAKE_VAULT_FIXTURE").expect("нет FAKE_VAULT_FIXTURE");
    serde_json::from_str(&std::fs::read_to_string(path).expect("нет фикстуры")).expect("фикстура не JSON")
}

fn session_ok() -> bool {
    std::env::var("BW_SESSION").is_ok_and(|v| v == SESSION)
}

fn fail(text: &str) -> ! {
    eprintln!("{text}");
    std::process::exit(1)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&args);
    // Программа всегда просит CLI не задавать вопросов: без этого флага вызов считается ошибкой.
    if !args.iter().any(|a| a == "--nointeraction") {
        fail("interactive prompt would be needed");
    }
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    match (arg(0), arg(1)) {
        ("status", _) => {
            let status = if session_ok() { "unlocked" } else { "locked" };
            println!("{}", json!({ "status": status, "userEmail": fixture()["bitwarden"]["email"] }));
        }
        ("unlock", _) => {
            if std::env::var("DELOIST_BW_PASS").ok().as_deref() != Some(PASSWORD) {
                fail("Invalid master password.");
            }
            println!("{SESSION}");
        }
        ("list", "items") => {
            if !session_ok() {
                fail("Vault is locked.");
            }
            println!("{}", fixture()["bitwarden"]["items"]);
        }
        ("get", "item") => {
            if !session_ok() {
                fail("Vault is locked.");
            }
            let id = arg(2);
            let item = fixture()["bitwarden"]["items"].as_array().and_then(|items| items.iter().find(|i| i["id"] == id).cloned());
            match item {
                Some(item) => println!("{item}"),
                None => fail("Not found."),
            }
        }
        ("get", "totp") => {
            if !session_ok() {
                fail("Vault is locked.");
            }
            let id = arg(2);
            let code = fixture()["bitwarden"]["items"]
                .as_array()
                .and_then(|items| items.iter().find(|i| i["id"] == id).and_then(|i| i["login"]["totp"].as_str().map(str::to_string)));
            match code {
                Some(code) if !code.is_empty() => println!("{code}"),
                _ => fail("Not found."),
            }
        }
        _ => fail("unknown command"),
    }
}
