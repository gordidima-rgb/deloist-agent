use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    // Не `%TEMP%`: он лежит в `AppData`, а такие папки команде не отдаются (`grants::protected_folder`).
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("unit-tmp").join(format!("deloist-{name}-{stamp}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn folder_validation_refuses_what_a_team_must_not_get() {
    let data = temp("rf-data");
    let work = temp("rf-work");
    let sub = work.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(work.join("file.txt"), "x").unwrap();
    let ok = check_folder(&data, work.to_str().unwrap()).unwrap();
    assert!(ok.to_lowercase().ends_with(&work.file_name().unwrap().to_string_lossy().to_lowercase()));
    let sep = std::path::MAIN_SEPARATOR;
    let dotted = format!("{}{sep}sub{sep}..", work.display());
    assert!(check_folder(&data, &dotted).is_err(), "..");
    assert!(check_folder(&data, "relative/dir").is_err());
    assert!(check_folder(&data, work.join("file.txt").to_str().unwrap()).is_err(), "файл");
    assert!(check_folder(&data, work.join("none").to_str().unwrap()).is_err(), "нет папки");
    assert!(check_folder(&data, data.to_str().unwrap()).is_err(), "служебная папка");
    assert!(check_folder(&data, data.parent().unwrap().to_str().unwrap()).is_err(), "накрывает служебную");
    assert!(check_folder(&data, "").is_err());
    assert!(check_folder(&data, &"a".repeat(300)).is_err());
    if cfg!(windows) {
        assert!(check_folder(&data, r"\\server\share\x").is_err(), "UNC");
        assert!(check_folder(&data, r"\\?\C:\Windows").is_err(), "префикс устройства");
        assert!(check_folder(&data, r"C:\").is_err(), "корень диска");
        assert!(check_folder(&data, r"C:\Windows").is_err(), "системная");
        // Хвостовая точка и регистр: та же папка, а не обход.
        let trailing = format!("{}.", work.display());
        if let Ok(real) = check_folder(&data, &trailing) {
            assert_eq!(real.to_lowercase(), ok.to_lowercase());
        }
        // Короткое имя 8.3 приводится к настоящему.
        let upper = work.to_str().unwrap().to_uppercase();
        if let Ok(real) = check_folder(&data, &upper) {
            assert_eq!(real.to_lowercase(), ok.to_lowercase());
        }
    }
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn folder_secrets_and_symlinks_are_refused() {
    let data = temp("rf-sec-data");
    let work = temp("rf-sec");
    let ssh = work.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    assert!(check_folder(&data, ssh.to_str().unwrap()).is_err(), ".ssh");
    let link = work.join("link");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&data, &link).is_ok();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&data, &link).is_ok();
    if made {
        assert!(check_folder(&data, link.to_str().unwrap()).is_err(), "симлинк на служебную папку");
    }
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn list_caps_are_checked() {
    let data = temp("rf-caps-data");
    let work = temp("rf-caps");
    let many: Vec<Value> = (0..=MAX_FOLDERS).map(|_| json!(work.to_str().unwrap())).collect();
    assert!(check_folders(&data, &json!(many)).is_err(), "слишком много");
    let twice = check_folders(&data, &json!([work.to_str().unwrap(), work.to_str().unwrap().to_uppercase()]));
    assert_eq!(twice.map(|l| l.len()).unwrap_or(2), 1, "повтор схлопывается");
    assert!(check_folders(&data, &json!("C:/x")).is_err(), "не список");
    assert!(check_folders(&data, &json!([5])).is_err(), "не строка");
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn room_folders_and_skills_apply_only_to_members_and_are_revoked() {
    let data = temp("rf-eff-data");
    let work = temp("rf-eff");
    let store = crate::store::Store::open(data.clone()).await.unwrap();
    let bot = store.add_bot(&json!({ "name": "Рен" })).await.unwrap();
    let other = store.add_bot(&json!({ "name": "Лис" })).await.unwrap();
    let own = store.add_skill(&json!({ "name": "Свой", "body": "a" })).await.unwrap();
    let shared = store.add_skill(&json!({ "name": "Общий", "body": "b" })).await.unwrap();
    store.update_bot(&bot.id, &json!({ "skills": [own.id.clone()] })).await.unwrap();
    let room = store.add_room(&json!({ "name": "Совет", "botIds": [bot.id.clone()] })).await.unwrap();
    let folder = check_folder(&data, work.to_str().unwrap()).unwrap();
    store.update_room(&room.id, &json!({ "folders": [folder.clone()], "skills": [shared.id.clone()] })).await.unwrap();

    let names = |list: Vec<crate::store::Skill>| list.into_iter().map(|s| s.name).collect::<Vec<_>>();
    assert_eq!(names(store.skills_for_bot_in_room(&bot.id, &room.id)), vec!["Свой", "Общий"]);
    assert_eq!(names(store.skills_for_bot(&bot.id)), vec!["Свой"], "вне группы общих навыков нет");
    assert!(store.skills_for_bot_in_room(&other.id, &room.id).is_empty(), "не участник");
    assert_eq!(homes_in_room(&store, &room.id, &bot.id).len(), 1);
    assert!(homes_in_room(&store, &room.id, &other.id).is_empty(), "не участник");

    // Убрали папку: доступ кончился. Ушёл бот или архив: тоже.
    store.update_room(&room.id, &json!({ "folders": [] })).await.unwrap();
    assert!(homes_in_room(&store, &room.id, &bot.id).is_empty());
    store.update_room(&room.id, &json!({ "folders": [folder] })).await.unwrap();
    store.update_room(&room.id, &json!({ "archived": true })).await.unwrap();
    assert!(homes_in_room(&store, &room.id, &bot.id).is_empty());
    store.update_room(&room.id, &json!({ "archived": false, "botIds": [other.id.clone()] })).await.unwrap();
    assert!(homes_in_room(&store, &room.id, &bot.id).is_empty());
    assert_eq!(names(store.skills_for_bot_in_room(&bot.id, &room.id)), vec!["Свой"]);

    // Папку убрали с диска: не отдаём. Навык удалён: из группы уходит тоже.
    store.update_room(&room.id, &json!({ "botIds": [bot.id.clone()] })).await.unwrap();
    std::fs::remove_dir_all(&work).unwrap();
    assert!(homes_in_room(&store, &room.id, &bot.id).is_empty());
    store.remove_skill(&shared.id).await.unwrap();
    assert!(store.room(&room.id).unwrap().skills.is_empty());
    let _ = std::fs::remove_dir_all(&data);
}

#[tokio::test]
async fn team_folders_follow_the_bot_into_every_conversation_but_only_while_it_is_a_member() {
    let data = temp("rf-any-data");
    let work = temp("rf-any");
    let store = crate::store::Store::open(data.clone()).await.unwrap();
    let bot = store.add_bot(&json!({ "name": "Рен" })).await.unwrap();
    let other = store.add_bot(&json!({ "name": "Лис" })).await.unwrap();
    let room = store.add_room(&json!({ "name": "Совет", "botIds": [bot.id.clone()] })).await.unwrap();
    let folder = check_folder(&data, work.to_str().unwrap()).unwrap();
    store.update_room(&room.id, &json!({ "folders": [folder.clone()] })).await.unwrap();
    // Без `conv_of`: личный чат бота тоже видит папку команды.
    let mine = homes_of_bot(&store, &bot.id);
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].team, "Совет");
    assert!(!mine[0].program);
    assert!(homes_of_bot(&store, &other.id).is_empty(), "не участник");
    store.update_room(&room.id, &json!({ "archived": true })).await.unwrap();
    assert!(homes_of_bot(&store, &bot.id).is_empty(), "архив");
    store.update_room(&room.id, &json!({ "archived": false, "botIds": [other.id.clone()] })).await.unwrap();
    assert!(homes_of_bot(&store, &bot.id).is_empty(), "вышел");
    assert_eq!(homes_of_bot(&store, &other.id).len(), 1);
    store.update_room(&room.id, &json!({ "folders": [] })).await.unwrap();
    assert!(homes_of_bot(&store, &other.id).is_empty(), "папку убрали");
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&work);
}

#[cfg(windows)]
#[tokio::test]
async fn a_team_folder_replaced_by_a_junction_is_dropped() {
    let data = temp("rf-junction-data");
    let work = temp("rf-junction");
    let elsewhere = temp("rf-junction-else");
    let store = crate::store::Store::open(data.clone()).await.unwrap();
    let bot = store.add_bot(&json!({ "name": "Рен" })).await.unwrap();
    let room = store.add_room(&json!({ "name": "Совет", "botIds": [bot.id.clone()] })).await.unwrap();
    let folder = check_folder(&data, work.to_str().unwrap()).unwrap();
    store.update_room(&room.id, &json!({ "folders": [folder] })).await.unwrap();
    assert_eq!(homes_of_bot(&store, &bot.id).len(), 1);
    // Папку удалили и на её место поставили ссылку на другую: настоящий путь теперь другой.
    std::fs::remove_dir_all(&work).unwrap();
    let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J", &work.display().to_string(), &elsewhere.display().to_string()]).output().map(|o| o.status.success()).unwrap_or(false);
    if made {
        assert!(homes_of_bot(&store, &bot.id).is_empty(), "подменённая ссылкой папка осталась доступной");
        let _ = std::process::Command::new("cmd").args(["/c", "rmdir", &work.display().to_string()]).output();
    }
    let _ = std::fs::remove_dir_all(&data);
    let _ = std::fs::remove_dir_all(&elsewhere);
}
