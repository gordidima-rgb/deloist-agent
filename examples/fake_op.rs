//! Заглушка 1Password CLI (`op`) для тестов менеджера паролей: только команды, которые использует программа.
//! Данные читаются из фикстуры `FAKE_VAULT_FIXTURE`. Токен берётся из `OP_SERVICE_ACCOUNT_TOKEN`.
//! Каждый запуск дописывает строку аргументов в `FAKE_VAULT_LOG`.

use std::io::Write;

use serde_json::{json, Value};

const TOKEN: &str = "ops_test_token";

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

fn fail(text: &str) -> ! {
    eprintln!("{text}");
    std::process::exit(1)
}

/// Значение поля по метке или по типу.
fn field_value(item: &Value, pick: impl Fn(&Value) -> bool) -> Option<String> {
    item["fields"].as_array()?.iter().find(|f| pick(f))?["value"].as_str().map(str::to_string)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&args);
    if std::env::var("OP_SERVICE_ACCOUNT_TOKEN").ok().as_deref() != Some(TOKEN) {
        fail("account is not signed in");
    }
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_default();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    match (arg(0), arg(1)) {
        ("whoami", _) => println!("{}", json!({ "user_type": "SERVICE_ACCOUNT", "account_uuid": "fake" })),
        ("item", "list") => println!("{}", fixture()["onepassword"]["items"]),
        ("item", "get") => {
            let id = arg(2);
            let details = &fixture()["onepassword"]["details"];
            let Some(item) = details.get(id) else {
                fail("item not found");
            };
            if has("--otp") {
                match field_value(item, |f| f["type"] == "OTP") {
                    Some(code) => println!("{code}"),
                    None => fail("no one-time password"),
                }
            } else if has("--fields") {
                match field_value(item, |f| f["label"] == "password") {
                    Some(value) => println!("{value}"),
                    None => fail("no password"),
                }
            } else {
                println!("{item}");
            }
        }
        _ => fail("unknown command"),
    }
}
