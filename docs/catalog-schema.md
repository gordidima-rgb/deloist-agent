# Каталог: формат манифеста и API

Основа — раздел «Формат манифеста» в `research/catalog/notes-integration.md` (schema 1). Здесь то, что каталог в коде принимает на деле, и **уточнения**, которых в заметке не было. Поля заметки не переименованы и не убраны.

## Папка

- `catalog/` в корне репозитория, встраивается в exe (`build.rs` → `catalog_files.rs`). Подпапки не важны: читаются все `*.json` (сейчас `skills/`, `agents/`, `apps/`).
- Один JSON — один элемент. `catalog/index.json` — список `{id, kind, category, title, version, path, hash}`. Хэш — sha256 текста файла с переводами строк LF.
- После любой правки манифеста: `node tools/catalog_index.mjs` (пересобрать индекс), `node tools/catalog_index.mjs --check` (только сверить). Юнит-тест `catalog::tests::the_builtin_catalog_is_valid_and_indexed` падает, если индекс устарел или манифест не прошёл проверку.

## Проверка (src/catalog.rs → `validate`, `build`)

| Правило | Почему |
|---|---|
| `schema` = 1 | |
| `id`: строчные латинские, цифры, `.`, `-`, `_`; **начинается с `<kind>.`** (`skill.…`, `agent.…`, `app.…`) — *уточнение* | id уникальны между видами, видно вид по id |
| `kind` ∈ skill, agent, app; `category` ∈ productivity, docs, research, business, marketing, dev, personal, design; `tier` ∈ verified, public | |
| `version` не пусто; `title.ru`, `summary.ru` не пусты | |
| `source.license` и `source.name` обязательны; `source.url`, если есть, — https | без лицензии элемент не попадает в каталог |
| навык: `skill.body.ru` не пуст | |
| агент: `agent.persona` не пуст; `agent.model_hint` обязателен; `agent.approval` ∈ ask, auto, strict (пусто = ask); `agent.skills` — id навыков **этого же каталога** | |
| приложение: `transport` = http (нужен https-`url`) или stdio (нужна `command`); без `transport` нужен `web_cabinet.url` (https) | |
| `install.kind` npx/uvx — обязательны `package` и `pin`, `pin` не `latest` и без `*`; binary — `sha256` из 64 hex | без `@latest` |
| `auth.fields[]`: у каждого `name` и `label` | |
| id не повторяются; индекс совпадает с файлами (id, версия, хэш) | |

Ошибочный элемент отбрасывается (в журнал и в `errors` ответа API), остальные работают.

## Уточнения к формату (новые необязательные поля)

- `description` `{ru, en}` — полный текст для боковой панели. Нет — показывается `summary`.
- Тексты агента `agent.role`, `agent.persona` — строка **или** `{ru, en}`; у специалиста `agent.persona` — объект с обоими языками.
- `agent.avatar`: `emoji`, `color` (`#rrggbb`), `shape`, `eyes`, `ears`, `character` — как в `Bot`.
- `agent.model_hint`: `strong` | `standard` | `fast`; старые `smart` → `strong`, `fast` → `fast` (нормализуются при загрузке). Модель не ставится: бот получает модель человека по умолчанию, подсказка видна в панели и лежит в `bot.extra.modelHint`.
- `agent.effort`: `low` | `medium` | `high` (по умолчанию `medium`).
- `agent.rank`: `lead` (по умолчанию) | `specialist`. У специалиста обязательны `agent.triggers` (`files[]` + `keywords{ru,en}`, каждый список 1–40) и `agent.checks` (2–3 задачи `{task{ru,en}, criteria{ru[],en[]}}`), а в `agent.persona.ru` и `.en` — восемь разделов по порядку (см. `docs/catalog-agent-template.md`).
- `agent.persona.{ru,en}` — до 12000 символов на язык.
- `app.egress[]` — хосты, куда уходит ключ (показываются в карточке согласия).
- `app.auth`: `kind` (`api_key` | `oauth` | `app_password` | `none`), `fields[]` = `{name, label{ru,en}, secret (по умолчанию true), help{ru,en}, help_url, header?, format?}`, `oauth` (свободный объект для модуля коннекторов).
- `app.web_cabinet`: `url`, `sites[]` (что добавить в `Bot.browserSites`; пусто — хост из `url`), `note{ru,en}`.

## Что хранит установка

- Навык → `Skill` с `origin = "catalog"`, `catalogId`, `catalogVersion`, `catalogHash` (хэш имени, описания и тела в момент установки или последнего обновления). Другой хэш = человек правил навык.
- Агент → `Bot` из пресета; `extra.catalogId`, `catalogVersion`, `catalogHash` (роль + характер), `modelHint`.
- Что добавила каждая установка — `catalog-installs.json` в папке данных: id навыка, бот, навыки, которых до установки не было, добавленные сайты, id подключения приложения и его статус. «Удалить» убирает ровно это.

## API

Все записи — только из окна приложения: ключ программы (`runtime.json`) получает 403 (`guard::catalog_write`). Чтение ключу открыто.

| Запрос | Что делает |
|---|---|
| `GET /api/catalog?kind=&category=&tier=&q=` | `{items, categories, origin, errors}`; у элемента `state`: `installed`, `version`, `update`, `edited`, `bots`/`botId`/`connectorId`, `status`; у приложений `mode`: `connector` или `cabinet` |
| `GET /api/catalog/{id}?lang=` | `{item}`; у агента `skillItems`, у приложения `card` (карточка согласия) |
| `POST /api/catalog/{id}/install` | навык `{bots[], lang}`; агент `{sites: bool, driver?, model?, lang}`; кабинет `{bot, consent: true}`; приложение `{}` → модуль коннекторов. Повторный вызов ничего не дублирует |
| `POST /api/catalog/{id}/connect` | `{step: confirm \| secret \| oauth \| check, name?, value?}`. Секрет только транзитом, поле должно быть из `auth.fields` |
| `POST /api/catalog/{id}/update` | `{resolve?: mine \| theirs}`. Правленный текст без `resolve` → 409 `{conflicts: [{what, id, title, mine, theirs}]}`. `theirs` кладёт правку человека в историю навыка (`/api/skills/{id}/undo` её вернёт) |
| `DELETE /api/catalog/{id}` | снимает то, что добавила установка; навыки агента остаются, если ими пользуется другой бот |

## Модуль коннекторов (ветка `feat/connectors-base`)

Пока его нет, `src/connectors_client.rs` ходит по HTTP туда, куда указывает `CREW_CONNECTORS_URL` (только debug и только loopback; в тестах там заглушка). Без адреса «Подключить» отвечает 503 с карточкой согласия из манифеста, и интерфейс честно пишет, что подключение появится вместе с модулем. Ожидаемый контракт (сверить с `docs/plans/2026-11-11-connectors-base.md`, когда он появится, и поменять клиент на прямые вызовы модуля):

| Шаг | Запрос | Ответ |
|---|---|---|
| установить | `POST /api/connectors/install {catalogId, manifest}` | `{id, card}` |
| согласие | `POST /api/connectors/{id}/confirm` | любое |
| ключ | `POST /api/connectors/{id}/secret {name, value}` | любое |
| OAuth | `POST /api/connectors/{id}/oauth/start` | `{url}` (https или loopback) |
| проверка | `POST /api/connectors/{id}/check` | `{tools: […]}` |
| удалить | `DELETE /api/connectors/{id}` | любое, 404 = уже нет |

## Обновление с deloist.ru

Не скачивается. Крючок — `catalog::verify_signed_index(index, signature)`: подпись ed25519 тем же ключом, что у выпусков (`update::verify`). Загрузчик должен проверить подпись, скачать файлы, сверить каждый с хэшем из индекса, собрать каталог через `catalog::build` и показывать diff текстов перед «Обновить». Для разработки `CREW_CATALOG_DIR` (только debug) подменяет встроенный каталог папкой целиком.
