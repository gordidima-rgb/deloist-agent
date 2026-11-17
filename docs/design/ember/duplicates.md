# Дубли кита: что рисуется мимо «Углей»

Снято 2026-11-17 grep-ом по `web/*.js|css|html` и `ui/src/**` (кроме `ui/src/components/kit`, `web/kit.css`, тестов).
Экраны здесь не переделаны: это план этапа B. Правило владельца: весь вид берётся из кита (`docs/design/kit.md`, правило 1).
Строки `файл:строка` — место; «→» — чем заменить. Проверка `tools/bots-kit-check-ember.mjs` держит файлы из раздела «Файлы-исключения» на предупреждениях, всё остальное — ошибки.

## Кнопки (свои классы и стили) (488)

Этап C2 (17.11): ✓ — файл переведён на `Button` / `IconButton` / `ListRow` / `Menu`, ◐ — переведена часть (остались строки-карточки, дорожка-полоса отчёта, микрофон и отправка у Stage, `AddMenu` — поиск с группами, а не меню).

Этап C3 (17.11): ✓ `web/messengers.js` (фишка и сегменты), `web/turn-card.js` (Стоп), `web/snap.html`, `web/dock.html` (кнопки чата и заметки), `web/voice.js` (свой `<style>` убран в блок C3 `web/kit.css`), `web/user-chrome.js`, `web/stt.js`; алиасы `.primary/.ghost/.danger` из `web/kit.css` удалены (нигде не используются). ◐ `web/app.js` (кнопки уже `k-btn`; остались `.icon-btn`, `.day-btn`, `.look-btn`, `.seg`), `web/index.html` и `web/style.css` — предупреждения 272 → 20, самодельные кнопки-классы остались.

Заменить на: `Button` / `IconButton` (`.k-btn`, варианты primary/secondary/ghost/danger/danger-solid)

- `ui/src/components/ChatModelPopover.svelte` — 1: 68 ✓ C2
- `ui/src/components/KnowMe.svelte` — 13: 144, 167, 168, 180, 181, 188, 195, 200, 201, 207, 229, 230, … ✓ C2
- `ui/src/components/MentionPopover.svelte` — 1: 99 ◐ C2 (часть)
- `ui/src/components/ModelPicker.svelte` — 11: 169, 178, 198, 205, 227, 246, 258, 450, 463, 467, 471 ◐ C2 (часть)
- `ui/src/components/NewBotModel.svelte` — 3: 49, 55, 60
- `ui/src/components/ScreenControl.svelte` — 3: 16, 17, 18 ✓ C2
- `ui/src/components/SignInCard.svelte` — 6: 27, 30, 31, 59, 69, 73 ✓ C2
- `ui/src/components/UpdatePlaque.svelte` — 1: 25 ✓ C2
- `ui/src/components/VoiceChat.svelte` — 3: 43, 44, 66 ✓ C2
- `ui/src/components/WhatsNew.svelte` — 2: 196, 212 ✓ C2
- `ui/src/components/catalog/CatalogCard.svelte` — 1: 16
- `ui/src/components/catalog/CatalogDrawer.svelte` — 2: 471, 505 ✓ C2
- `ui/src/components/chat/ChatDates.svelte` — 1: 133
- `ui/src/components/drawer/DrawerFiles.svelte` — 2: 81, 84 ✓ C2
- `ui/src/components/drawer/DrawerProfile.svelte` — 4: 385, 395, 410, 420 ✓ C2
- `ui/src/components/drawer/FileViewer.svelte` — 5: 197, 198, 287, 296, 297 ✓ C2
- `ui/src/components/orchestrator/Stage.svelte` — 5: 427, 458, 482, 493, 496 ◐ C2 (часть)
- `ui/src/components/providers/ProviderList.svelte` — 1: 32 ✓ C2
- `ui/src/components/remote/RemoteChat.svelte` — 1: 192 ✓ C2
- `ui/src/components/remote/RemotePcs.svelte` — 1: 118
- `ui/src/components/report/Report.svelte` — 4: 113, 117, 146, 179 ◐ C2 (часть)
- `ui/src/components/rooms/RoomResources.svelte` — 1: 55 ✓ C2
- `ui/src/components/rooms/TeamScreen.svelte` — 3: 221, 225, 229
- `ui/src/components/routines/ChannelChips.svelte` — 1: 18
- `ui/src/components/routines/Composer.svelte` — 1: 182
- `ui/src/components/routines/HabitsTab.svelte` — 3: 228, 247, 275 ◐ C2 (часть)
- `ui/src/components/routines/RemindersTab.svelte` — 1: 248 ◐ C2 (часть)
- `ui/src/components/routines/TodosTab.svelte` — 5: 397, 432, 442, 458, 532 ◐ C2 (часть)
- `ui/src/components/settings/SettingsHint.svelte` — 1: 9
- `ui/src/components/sidebar/BotList.svelte` — 4: 339, 424, 437, 438
- `ui/src/components/sidebar/InstaboxCard.svelte` — 1: 25
- `ui/src/components/sidebar/NeedsBoard.svelte` — 1: 17
- `ui/src/components/usage/UsageScreen.svelte` — 6: 149, 151, 162, 279, 334, 340
- `ui/src/components/workflows/BotPicker.svelte` — 7: 39, 71, 89, 92, 95, 125, 141
- `ui/src/components/workflows/ContextMenu.svelte` — 1: 46 ◐ C2 (часть)
- `ui/src/components/workflows/HistoryPopover.svelte` — 5: 72, 79, 88, 98, 102 ◐ C2 (часть)
- `ui/src/components/workflows/LinkHint.svelte` — 1: 28
- `ui/src/components/workflows/ToolOptions.svelte` — 11: 42, 48, 55, 63, 67, 73, 179, 195, 200, 204, 208
- `ui/src/components/workflows/TriggerFields.svelte` — 7: 58, 74, 84, 187, 208, 224, 244
- `ui/src/components/workflows/WfEdge.svelte` — 3: 33, 104, 121
- `ui/src/components/workflows/WfNode.svelte` — 32: 100, 133, 134, 138, 144, 153, 162, 170, 182, 183, 189, 201, …
- `ui/src/components/workflows/WfNote.svelte` — 3: 126, 136, 139
- `ui/src/components/workflows/WorkflowCanvas.svelte` — 25: 770, 771, 777, 807, 810, 813, 816, 826, 835, 836, 837, 838, …
- `ui/src/components/workflows/WorkflowList.svelte` — 7: 43, 47, 63, 328, 344, 348, 351
- `ui/src/components/workflows/WorkflowsScreen.svelte` — 10: 362, 400, 401, 425, 429, 479, 488, 528, 544, 547
- `web/app.js` — 33: 722, 1194, 2024, 3162, 3164, 3168, 3169, 3235, 3236, 3335, 3336, 3363, …
- `web/dock.html` — 4: 315, 385, 386, 392
- `web/index.html` — 83: 19, 20, 21, 25, 26, 38, 39, 40, 42, 43, 48, 53, …
- `web/messengers.js` — 3: 94, 154, 156
- `web/providers.js` — 6: 184, 212, 240, 272, 317, 668
- `web/screen.js` — 13: 554, 555, 559, 562, 567, 568, 569, 582, 584, 593, 594, 595, …
- `web/signin.js` — 12: 37, 41, 42, 51, 54, 55, 64, 65, 236, 239, 388, 392
- `web/snap.html` — 4: 38, 47, 52, 53
- `web/style.css` — 118: 162, 163, 209, 223, 224, 307, 308, 309, 319, 320, 321, 343, …
- `web/turn-card.js` — 1: 86

<details><summary>Примеры строк</summary>

```
web/app.js:722  chip.innerHTML = `<button type="button" class="guest-open"></button><button type="button" class="guest-drop" a
web/app.js:1194  <button type="button" class="ghost made-open">${tr('Открыть')}</button>
web/app.js:2024  ${demo ? `<p class="hint">${tr('Сейчас демо-режим: ответы идут по сценарию.')}</p><button type="button" class=
web/app.js:3162  <div class="card-actions"><button class="primary" data-send>${tr('Ответить')}</button><button class="ghost" da
web/app.js:3164  const choices = (ev.choices || []).map((c) => `<button class="ghost" data-choice="${escapeHtml(c)}">${escapeHt
web/app.js:3168  <button class="primary" data-send>${tr('Ответить')}</button>
web/app.js:3169  <button class="ghost" data-dismiss title="${tr('Убрать вопрос')}" aria-label="${tr('Убрать вопрос')}">×</butto
web/app.js:3235  <button class="primary" data-accept>${tr('Передать')}</button>
web/app.js:3236  <button class="ghost" data-decline>${tr('Сам справлюсь')}</button>
web/app.js:3335  <button class="primary" data-accept>${tr('Применить')}</button>
web/app.js:3336  <button class="ghost" data-decline>${tr('Не надо')}</button>
web/app.js:3363  <button class="primary" data-accept>${tr('Создать')}</button>
web/app.js:3364  <button class="ghost" data-decline>${tr('Не надо')}</button>
web/app.js:3421  <button class="primary" data-accept>${tr('Создать')}</button>
web/app.js:3422  <button class="ghost" data-decline>${tr('Не надо')}</button>
web/app.js:3466  <button class="primary" data-accept>${tr('Написать скрипт')}</button>
web/app.js:3467  <button class="ghost" data-decline>${tr('Разовая работа')}</button>
web/app.js:4352  <div class="m-actions"><button class="icon-btn" data-del title="${tr('Удалить скрипт')}" aria-label="${tr('Уда
web/app.js:4389  <button type="button" class="ghost">${tr('Убрать')}</button>`;
web/app.js:4502  <button type="button" class="ghost" data-reject-all>${tr('Отклонить все и не принимать заявки 1 час')}</button
web/app.js:4503  <button type="button" class="ghost" data-decline>${tr('Отклонить')}</button>
web/app.js:4504  <button type="button" class="primary" data-accept>${tr('Добавить')}</button>
web/app.js:6059  <button type="button" class="primary" id="drawer-rt-new">${tr('Новая рутина')}</button>`;
web/app.js:6186  <button type="button" class="ghost" data-cancel>${tr('Отмена')}</button>
web/app.js:6187  <button type="submit" class="primary">${tr('Сохранить')}</button>
```
</details>

## Выпадающие списки (52)

Заменить на: `Select` (`kit/inputs`) — свой список; нативный `<select>` не рисовать

- `ui/src/components/ChatModelPopover.svelte` — 1: 85 ✓ C2
- `ui/src/components/VoiceChat.svelte` — 2: 47, 68 ✓ C2
- `ui/src/components/catalog/CatalogDrawer.svelte` — 1: 270 ✓ C2
- `ui/src/components/workflows/ScheduleEditor.svelte` — 3: 27, 44, 78
- `ui/src/components/workflows/WfNode.svelte` — 1: 280
- `ui/src/components/workflows/WfNote.svelte` — 2: 69, 79
- `ui/src/components/workflows/WfShape.svelte` — 1: 66
- `ui/src/components/workflows/WorkflowCanvas.svelte` — 1: 990
- `web/app.js` — 2: 3153, 6182
- `web/index.html` — 16: 217, 268, 272, 354, 385, 395, 396, 411, 504, 555, 696, 703, …
- `web/style.css` — 22: 162, 1158, 1168, 1242, 1252, 1254, 1255, 1260, 2426, 2441, 2454, 2459, …

<details><summary>Примеры строк</summary>

```
web/app.js:3153  return `<label class="field"><span>${label}</span><select data-field="${name}"><option value=""></option>${opt
web/app.js:6182  <label class="field"><span>${tr('Тип')}</span><select name="kind">${kinds}</select></label>
web/index.html:217  <select name="team" id="nb-team"></select>
web/index.html:268  <select name="lead" id="room-lead"></select>
web/index.html:272  <select name="mode" id="room-mode">
web/index.html:354  <select name="skillLearning" id="skill-learning" class="s-select" aria-label="Самообучение навыкам">
web/index.html:385  <select name="driver">
web/index.html:395  <select name="acpModel"></select>
web/index.html:396  <select name="planModel">
web/index.html:411  <select name="fallbackDriver" class="s-select" aria-label="Запасная модель">
web/index.html:504  <select name="petsMode" id="pets-mode" class="s-select" aria-label="Где показывать ботов">
web/index.html:555  <select name="browserEngine" id="browser-engine" class="s-select" aria-label="Чем бот открывает сайты">
web/index.html:696  <select id="dock-layer" class="dock-select">
web/index.html:703  <select id="dock-display" class="dock-select"></select>
web/index.html:707  <select id="dock-edge" class="dock-select">
web/index.html:805  <select id="mcp-transport" data-testid="mcp-transport">
web/index.html:1017  <select name="botId" id="routine-bot"></select>
web/index.html:1021  <select name="mode" id="routine-mode">
web/style.css:162  button, input, textarea, select { font: inherit; color: inherit; }
web/style.css:1158  #drawer .field select { height: 40px; }
web/style.css:1168  #drawer .field select {
web/style.css:1242  .field input, .field textarea, .field select, #mem-text {
web/style.css:1252  .field input, .field select { height: 40px; }
web/style.css:1254  .field input:hover, .field textarea:hover, .field select:hover { border-color: var(--color-border-strong); }
web/style.css:1255  .field input:focus, .field textarea:focus, .field select:focus, #mem-text:focus {
```
</details>

## Галочки, радио, переключатели (88)

Заменить на: `Checkbox` / `RadioGroup` / `Switch`

- `ui/src/components/KnowMe.svelte` — 2: 159, 223
- `ui/src/components/ModelPicker.svelte` — 3: 586, 594, 598
- `ui/src/components/catalog/CatalogDrawer.svelte` — 3: 230, 248, 275
- `ui/src/components/drawer/DrawerGoals.svelte` — 1: 127
- `ui/src/components/drawer/DrawerProfile.svelte` — 1: 533
- `ui/src/components/rooms/RoomMembers.svelte` — 2: 3, 43
- `ui/src/components/rooms/RoomResources.svelte` — 1: 100
- `ui/src/components/routines/HabitsTab.svelte` — 5: 530, 544, 547, 550, 611
- `ui/src/components/routines/TodosTab.svelte` — 5: 805, 819, 822, 829, 887
- `ui/src/components/sidebar/BotList.svelte` — 3: 386, 387, 388
- `ui/src/components/workflows/WfNode.svelte` — 3: 846, 859, 863
- `web/app.js` — 5: 6503, 6594, 7159, 7217, 7780
- `web/index.html` — 12: 366, 469, 490, 591, 626, 633, 714, 718, 722, 824, 1069, 1073
- `web/providers.js` — 2: 180, 197
- `web/style.css` — 40: 786, 1183, 1185, 1347, 1348, 1407, 1408, 1409, 1417, 1428, 1429, 1602, …

<details><summary>Примеры строк</summary>

```
web/app.js:6503  el.innerHTML = `<input type="checkbox" ${on.has(bot.id) ? 'checked' : ''} ${selfDev?.effective ? '' : 'disable
web/app.js:6594  <label class="switch"><input type="checkbox" ${s.enabled ? 'checked' : ''}><span></span></label>
web/app.js:7159  return `<label class="skill-check"${blocked ? ` title="${escapeHtml(limit)}"` : ''}><input type="checkbox" val
web/app.js:7217  `<label class="prov-line"><div class="dock-bot"><div class="dock-bot-face" data-bot="${escapeHtml(b.id)}"></di
web/app.js:7780  <label class="switch"><input type="checkbox" ${r.enabled ? 'checked' : ''}><span></span></label>
web/index.html:366  <span class="switch"><input type="checkbox" name="autostart" /><span></span></span>
web/index.html:469  <span class="switch"><input type="checkbox" name="vision" /><span></span></span>
web/index.html:490  <span class="switch"><input type="checkbox" id="dock-enabled" /><span></span></span>
web/index.html:591  <span class="switch"><input type="checkbox" name="keepAwake" /><span></span></span>
web/index.html:626  <span class="switch"><input type="checkbox" name="checkUpdates" /><span></span></span>
web/index.html:633  <span class="switch"><input type="checkbox" name="autoUpdate" /><span></span></span>
web/index.html:714  <span class="switch"><input type="checkbox" id="dock-autohide" /><span></span></span>
web/index.html:718  <span class="switch"><input type="checkbox" id="dock-activity" /><span></span></span>
web/index.html:722  <span class="switch"><input type="checkbox" id="dock-results" /><span></span></span>
web/index.html:824  <input type="checkbox" id="mcp-auto" />
web/index.html:1069  <input type="checkbox" name="quiet" checked />
web/index.html:1073  <input type="checkbox" name="hook" id="rt-hook" />
web/providers.js:180  <span class="switch"><input type="checkbox" data-model="${esc(m.id)}"${hidden.has(m.id) ? '' : ' checked'} /><
web/providers.js:197  + `<label class="prov-line"><div><div class="prov-line-title">${tr('Использовать {name}', { name })}</div><div
web/style.css:786  .model-switch { color: var(--color-text-muted); font-size: var(--text-sm); }
web/style.css:1183  #drawer .skill-check { display: flex; align-items: center; gap: var(--k-space-2); color: var(--k-text-2); font
web/style.css:1185  #drawer .skill-check input { width: 16px; height: 16px; accent-color: var(--k-brand); }
web/style.css:1347  .quiet-check { display: flex; align-items: center; gap: 8px; font-size: var(--text-sm); color: var(--color-tex
web/style.css:1348  .quiet-check input { width: 16px; height: 16px; accent-color: var(--color-brand-primary); }
web/style.css:1407  .switch { position: relative; width: 36px; height: 20px; flex: none; }
```
</details>

## Ползунки (3)

Заменить на: `Slider`

- `ui/src/components/VoiceChat.svelte` — 1: 53
- `ui/src/components/report/Report.svelte` — 1: 204
- `web/ui-demo.html` — 1: 74

<details><summary>Примеры строк</summary>

```
web/ui-demo.html:74  <label>Значение первого кольца <input type="range" id="slider" min="0" max="100" value="42" /></label>
ui/src/components/report/Report.svelte:204  .rp-range { font: 700 var(--k-text-sm, 13px) / 1.2 var(--k-font-body, inherit); min-width: 9ch; text-align: ce
ui/src/components/VoiceChat.svelte:53  <input type="range" min="0.8" max="1.5" step="0.05" value={speed}
```
</details>

## Поля ввода (130)

Заменить на: `TextInput` / `TextArea` / `Field` / `FieldRow` (`.k-input`, `.k-field`)

- `ui/src/components/KnowMe.svelte` — 1: 232
- `ui/src/components/ModelPicker.svelte` — 1: 526
- `ui/src/components/catalog/ConnectFlow.svelte` — 2: 204, 211
- `ui/src/components/devices/PairPlaques.svelte` — 1: 186
- `ui/src/components/drawer/DrawerProfile.svelte` — 1: 466
- `ui/src/components/drawer/FileViewer.svelte` — 2: 369, 380
- `ui/src/components/orchestrator/Stage.svelte` — 2: 659, 674
- `ui/src/components/providers/AgentCard.svelte` — 1: 300
- `ui/src/components/remote/RemoteChat.svelte` — 2: 306, 310
- `ui/src/components/rooms/RoomResources.svelte` — 1: 116
- `ui/src/components/routines/Composer.svelte` — 7: 199, 206, 210, 214, 230, 241, 315
- `ui/src/components/routines/HabitsTab.svelte` — 1: 382
- `ui/src/components/routines/TodoCard.svelte` — 1: 266
- `ui/src/components/routines/TodosTab.svelte` — 3: 591, 663, 673
- `ui/src/components/settings/VaultSettings.svelte` — 1: 395
- `ui/src/components/workflows/AddMenu.svelte` — 1: 159
- `ui/src/components/workflows/OutputFields.svelte` — 3: 58, 69, 73
- `ui/src/components/workflows/ScheduleEditor.svelte` — 2: 112, 124
- `ui/src/components/workflows/TriggerFields.svelte` — 3: 165, 176, 182
- `ui/src/components/workflows/WfNode.svelte` — 4: 440, 460, 464, 471
- `ui/src/components/workflows/WorkflowCanvas.svelte` — 1: 1332
- `web/dock.html` — 2: 339, 352
- `web/style.css` — 87: 162, 169, 245, 256, 257, 787, 796, 1049, 1064, 1065, 1066, 1067, …

<details><summary>Примеры строк</summary>

```
web/dock.html:339  .chat-input {
web/dock.html:352  .chat-input::placeholder { color: var(--color-text-muted); }
web/style.css:162  button, input, textarea, select { font: inherit; color: inherit; }
web/style.css:169  input, textarea { caret-color: var(--color-brand-primary); }
web/style.css:245  .search input {
web/style.css:256  .search input::placeholder { color: var(--color-text-muted); }
web/style.css:257  .search input:focus { border-color: var(--color-brand-primary); box-shadow: 0 0 0 3px var(--color-brand-tint);
web/style.css:787  .q-input {
web/style.css:796  .q-input:focus { border-color: var(--color-brand-primary); box-shadow: 0 0 0 3px var(--color-brand-tint); }
web/style.css:1049  .composer textarea {
web/style.css:1064  .composer textarea::placeholder { color: var(--k-text-3); }
web/style.css:1065  .composer textarea:hover:not(:disabled) { border-color: var(--k-text-3); }
web/style.css:1066  .composer textarea:focus { border-color: var(--k-brand); box-shadow: var(--k-focus); }
web/style.css:1067  .composer textarea:disabled { opacity: 0.6; }
web/style.css:1142  #drawer .field { gap: 6px; min-width: 0; }
web/style.css:1143  #drawer .field > span { color: var(--k-text-2); font: 600 var(--k-text-xs) / 1.3 var(--k-font-body); }
web/style.css:1146  #drawer .field textarea {
web/style.css:1158  #drawer .field select { height: 40px; }
web/style.css:1159  #drawer .field textarea { padding: var(--k-space-3); line-height: var(--k-leading-body); resize: vertical; }
web/style.css:1161  #drawer .field textarea::placeholder { color: var(--k-text-3); }
web/style.css:1164  #drawer .field textarea:hover { border-color: var(--k-text-3); }
web/style.css:1167  #drawer .field textarea:focus { border-color: var(--k-brand); box-shadow: var(--k-focus); }
web/style.css:1168  #drawer .field select {
web/style.css:1185  #drawer .skill-check input { width: 16px; height: 16px; accent-color: var(--k-brand); }
web/style.css:1240  .field { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
```
</details>

## Фишки и метки (110)

Заменить на: `Chip` (фильтр) или `Badge` (метка, счётчик, статус)

- `ui/src/components/NewBotModel.svelte` — 4: 100, 115, 118, 122
- `ui/src/components/catalog/YourSkills.svelte` — 1: 124
- `ui/src/components/chat/ChatDates.svelte` — 10: 171, 180, 183, 194, 197, 200, 203, 215, 219, 230
- `ui/src/components/routines/ChannelChips.svelte` — 1: 29
- `ui/src/components/routines/Composer.svelte` — 9: 259, 265, 276, 279, 282, 285, 297, 301, 312
- `ui/src/components/routines/TodosTab.svelte` — 3: 860, 867, 871
- `ui/src/components/sidebar/NeedsBoard.svelte` — 1: 36
- `ui/src/components/workflows/HistoryPopover.svelte` — 4: 192, 201, 206, 211
- `ui/src/components/workflows/KindChips.svelte` — 6: 29, 47, 51, 54, 61, 74
- `ui/src/components/workflows/WorkflowCanvas.svelte` — 8: 1112, 1126, 1130, 1135, 1139, 1144, 1505, 1508
- `ui/src/components/workflows/WorkflowList.svelte` — 5: 261, 273, 279, 284, 289
- `web/dock.html` — 20: 166, 167, 168, 181, 182, 183, 184, 185, 186, 187, 188, 189, …
- `web/style.css` — 34: 469, 470, 474, 486, 500, 501, 502, 503, 765, 1642, 1787, 1966, …
- `web/ui-demo.html` — 4: 43, 44, 54, 55

<details><summary>Примеры строк</summary>

```
web/dock.html:166  .pill-host { position: absolute; right: calc(var(--k-dock-width) - 1px); height: 52px; }
web/dock.html:167  body.left .pill-host { right: auto; left: calc(var(--k-dock-width) - 1px); }
web/dock.html:168  .pill {
web/dock.html:181  .pill.left { right: auto; left: 0; border-radius: 0 20px 20px 0; }
web/dock.html:182  .pill.leaving { opacity: 0; transition: width var(--k-dur-exit) var(--k-ease-in), opacity var(--k-dur-exit) va
web/dock.html:183  .pill::before, .pill::after { content: ""; position: absolute; right: 0; width: var(--join); height: var(--joi
web/dock.html:184  .pill::before { bottom: 100%; background: radial-gradient(circle at 0 0, transparent calc(var(--join) - 0.5px)
web/dock.html:185  .pill::after { top: 100%; background: radial-gradient(circle at 0 100%, transparent calc(var(--join) - 0.5px),
web/dock.html:186  .pill.left::before, .pill.left::after { right: auto; left: 0; }
web/dock.html:187  .pill.left::before { background: radial-gradient(circle at 100% 0, transparent calc(var(--join) - 0.5px), var(
web/dock.html:188  .pill.left::after { background: radial-gradient(circle at 100% 100%, transparent calc(var(--join) - 0.5px), va
web/dock.html:189  .pill-lines { position: absolute; inset: 0; overflow: hidden; border-radius: inherit; }
web/dock.html:190  .pill-line { position: absolute; left: 16px; right: 12px; display: flex; align-items: center; gap: 9px; white-
web/dock.html:191  .pill.left .pill-line { left: 14px; right: 16px; }
web/dock.html:192  .pill-line.now { top: 24px; font: 600 14px/22px var(--k-font-body); }
web/dock.html:193  .pill-line.was { top: 6px; font: 400 12px/18px var(--k-font-body); color: var(--k-on-ink-3); }
web/dock.html:194  .pill-meta { color: var(--k-on-ink-3); font-weight: 400; }
web/dock.html:195  .pill-text { overflow: hidden; text-overflow: ellipsis; }
web/dock.html:196  .pill-measure { position: absolute; visibility: hidden; white-space: nowrap; font: 600 14px/22px var(--k-font-
web/dock.html:373  .pill { transition: none; }
web/style.css:469  .guest-chip { display: inline-flex; align-items: center; flex: 0 1 auto; min-width: 56px; height: 26px; border
web/style.css:470  .guest-chip button { height: 100%; border: 0; background: none; color: inherit; cursor: pointer; }
web/style.css:474  .guest-chip button:focus-visible { outline: none; box-shadow: var(--k-focus); }
web/style.css:486  .chip {
web/style.css:500  .chip:focus-visible { outline: none; box-shadow: var(--k-focus); }
```
</details>

## Подсказки (3)

Заменить на: `Tooltip` / `InfoTip`

- `ui/src/components/Dock.svelte` — 2: 395, 401
- `web/style.css` — 1: 2735

<details><summary>Примеры строк</summary>

```
web/style.css:2735  #modal-settings .k-tooltip { white-space: normal; width: 280px; max-width: 70vw; z-index: 50; }
ui/src/components/Dock.svelte:395  .dock-voice-tip {
ui/src/components/Dock.svelte:401  .dock-voice-tip.left {
```
</details>

## Уведомления (3)

Заменить на: `Toast` / `ToastHost`

- `web/style.css` — 3: 1486, 2845, 2846

<details><summary>Примеры строк</summary>

```
web/style.css:1486  .toast {
web/style.css:2845  .toast { max-width: min(92vw, 720px); }
web/style.css:2846  .toast .ghost {
```
</details>

## Счётчики и значки (59)

Заменить на: `Badge` (счётчик: lava / neutral / dot)

- `ui/src/components/KnowMe.svelte` — 1: 227
- `ui/src/components/MentionPopover.svelte` — 1: 137
- `ui/src/components/ModelPicker.svelte` — 1: 506
- `ui/src/components/WhatsNew.svelte` — 1: 379
- `ui/src/components/catalog/Catalog.svelte` — 1: 248
- `ui/src/components/drawer/DrawerProfile.svelte` — 2: 478, 479
- `ui/src/components/providers/ProviderList.svelte` — 2: 105, 106
- `ui/src/components/rooms/TeamScreen.svelte` — 1: 299
- `ui/src/components/routines/TodosTab.svelte` — 1: 763
- `ui/src/components/settings/VaultSettings.svelte` — 2: 431, 438
- `ui/src/components/sidebar/BotList.svelte` — 1: 391
- `ui/src/components/sidebar/NeedsBoard.svelte` — 1: 31
- `ui/src/components/workflows/WfNode.svelte` — 1: 338
- `web/dock.html` — 1: 52
- `web/pet.html` — 2: 40, 48
- `web/style.css` — 40: 276, 279, 280, 297, 298, 325, 881, 1370, 1760, 1857, 1868, 1869, …

<details><summary>Примеры строк</summary>

```
web/dock.html:52  .wrap .avatar-badges { position: absolute; left: -2px; top: -2px; display: flex; gap: 1px; z-index: 1; }
web/pet.html:40  .pet-badges {
web/pet.html:48  .pet-badges .avatar-badge { border-color: transparent; }
web/style.css:276  .avatar:has(.avatar-badges) { position: relative; }
web/style.css:279  .avatar-badges { position: absolute; right: -5px; bottom: -4px; z-index: 1; display: flex; gap: 1px; }
web/style.css:280  .avatar-badge {
web/style.css:297  .avatar-badge.is-error { padding: 0; background: var(--color-accent-strong); color: var(--color-on-accent); }
web/style.css:298  .avatar-badge svg.i { width: 10px; height: 10px; stroke-width: 3.5; }
web/style.css:325  .badge { min-width: 8px; height: 8px; border-radius: 50%; background: var(--color-accent); }
web/style.css:881  .queue-count { color: var(--color-text-muted); font-size: var(--text-xs); font-variant-numeric: tabular-nums; 
web/style.css:1370  .side-foot .badge { margin-left: 2px; }
web/style.css:1760  .account-who { display: flex; flex-direction: column; gap: 2px; margin: 0; font-weight: 600; color: var(--colo
web/style.css:1857  .account-balance {
web/style.css:1868  .account-balance[hidden] { display: none; }
web/style.css:1869  .account-balance b { font-family: var(--font-heading); font-weight: 700; font-size: var(--text-lg); color: var
web/style.css:1870  .account-topup { display: flex; flex-direction: column; gap: var(--space-2); }
web/style.css:1871  .account-topup[hidden] { display: none; }
web/style.css:1872  .account-topup > span { font-size: var(--text-sm); font-weight: 600; color: var(--color-text-secondary); }
web/style.css:1873  .account-topup-sums { display: flex; gap: var(--space-2); flex-wrap: wrap; }
web/style.css:1874  .account-topup-sums .ghost { min-width: 88px; justify-content: center; }
web/style.css:1882  .prov-badge { display: inline-flex; align-items: center; gap: 5px; flex: none; padding: 2px 8px; border-radius
web/style.css:1883  .prov-badge::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
web/style.css:1884  .prov-badge[data-status="connected"] { background: var(--color-brand-tint); color: var(--color-brand-text); bo
web/style.css:1885  .prov-badge[data-status="not_installed"], .prov-badge[data-status="error"], .prov-badge[data-status="update"] 
web/style.css:1967  .msg-badge { display: grid; place-items: center; width: 28px; height: 28px; border-radius: 50%; border: 1px so
```
</details>

## Свои height/padding у кнопок (82)

Заменить на: убрать свои height/padding: размеры даёт `Button size="sm|md"` (30 / 38 px)

- `ui/src/components/KnowMe.svelte` — 1: 229
- `ui/src/components/ModelPicker.svelte` — 2: 453, 465
- `ui/src/components/ScreenControl.svelte` — 1: 16
- `ui/src/components/SignInCard.svelte` — 2: 60, 61
- `ui/src/components/VoiceChat.svelte` — 1: 66
- `ui/src/components/drawer/FileViewer.svelte` — 1: 288
- `ui/src/components/sidebar/BotList.svelte` — 2: 428, 430
- `ui/src/components/usage/UsageScreen.svelte` — 1: 340
- `ui/src/components/workflows/BotPicker.svelte` — 4: 76, 77, 130, 131
- `ui/src/components/workflows/LinkHint.svelte` — 1: 29
- `ui/src/components/workflows/ToolOptions.svelte` — 2: 183, 184
- `ui/src/components/workflows/TriggerFields.svelte` — 2: 209, 210
- `ui/src/components/workflows/WfEdge.svelte` — 2: 108, 109
- `ui/src/components/workflows/WfNode.svelte` — 6: 419, 604, 605, 670, 671, 778
- `ui/src/components/workflows/WfNote.svelte` — 2: 128, 129
- `ui/src/components/workflows/WorkflowCanvas.svelte` — 2: 1448, 1449
- `ui/src/components/workflows/WorkflowList.svelte` — 3: 331, 332, 333
- `ui/src/components/workflows/WorkflowsScreen.svelte` — 3: 480, 532, 533
- `web/dock.html` — 1: 317
- `web/snap.html` — 1: 38
- `web/style.css` — 42: 211, 220, 307, 319, 343, 355, 356, 359, 439, 470, 883, 1223, …

<details><summary>Примеры строк</summary>

```
web/dock.html:317  .m-note button → padding: 4px 10px;
web/snap.html:38  .actions button → .actions button { height: 32px; padding: 0 12px; border-radius: 10px; font-size: 13px; }
web/style.css:211  .icon-btn → height: 32px;
web/style.css:220  .icon-btn → line-height: 1;
web/style.css:307  .seg button → .seg button { height: 28px; padding: 0 14px; border: none; border-radius: var(--radius-pill); background: tran
web/style.css:319  .look-btn → .look-btn { display: grid; place-items: center; width: 48px; height: 48px; padding: 0; border: 1.5px solid tra
web/style.css:343  .side-foot .ghost → .side-foot .ghost { height: 36px; padding: 0 10px; border-color: transparent; background: transparent; font-si
web/style.css:355  .primary, .ghost, .danger → height: 40px;
web/style.css:356  .primary, .ghost, .danger → padding: 0 18px;
web/style.css:359  .primary, .ghost, .danger → line-height: 1;
web/style.css:439  .team-section-head .ghost → .team-section-head .ghost { height: 32px; padding: 0 var(--space-3); }
web/style.css:470  .guest-chip button → .guest-chip button { height: 100%; border: 0; background: none; color: inherit; cursor: pointer; }
web/style.css:883  .queue-head .ghost → .queue-head .ghost { height: 24px; padding: 0 10px; font-size: var(--text-xs); }
web/style.css:1223  #drawer .icon-btn → #drawer .icon-btn { width: 32px; height: 32px; border-radius: 50%; color: var(--k-text-3); }
web/style.css:1355  .day-btn → height: 30px;
web/style.css:1356  .day-btn → min-width: 38px;
web/style.css:1357  .day-btn → padding: 0 10px;
web/style.css:1391  .jr-missed .ghost → .jr-missed .ghost { height: 28px; padding: 0 10px; font-size: var(--text-sm); }
web/style.css:1459  .update-head .icon-btn → .update-head .icon-btn { width: 28px; height: 28px; margin: -2px -4px 0 0; }
web/style.css:1460  .update-head .icon-btn .i → .update-head .icon-btn .i { width: 16px; height: 16px; }
web/style.css:1463  .update-card .primary → .update-card .primary { margin-top: var(--space-2); height: 32px; padding: 0 var(--space-3); }
web/style.css:1724  .signin-providers .ghost → .signin-providers .ghost { min-width: 140px; height: 44px; justify-content: center; gap: var(--space-2); }
web/style.css:1874  .account-topup-sums .ghost → .account-topup-sums .ghost { min-width: 88px; justify-content: center; }
web/style.css:1911  button.link → button.link { border: 0; background: none; padding: 0; color: var(--color-brand-text); cursor: pointer; font: 
web/style.css:2052  .mem-fact-actions .ghost, .mem-foot .ghost → .mem-fact-actions .ghost, .mem-foot .ghost { height: 28px; padding: 0 10px; font-size: var(--text-sm); }
```
</details>

## Литеральные цвета, радиусы, шрифты по файлам

Цвета — в токены `--k-*`; радиусы — `--k-radius-*` (6 · 10 · 14 · 20 · 24 · 30); шрифты — `--k-text-*` / `--k-font-*`.

| файл | цветов | радиусов не из токенов | размеров шрифта не из токенов | первые строки (цвет) |
|---|---|---|---|---|
| `web/style.css` | 56 | 137 | 53 | 166, 168, 269, 758 |
| `ui/src/components/workflows/WfNode.svelte` | 7 | 25 | 23 | 272, 273, 437, 791 |
| `ui/src/components/routines/TodosTab.svelte` | 45 | 0 | 1 | 566, 574, 576, 607 |
| `ui/src/components/routines/HabitsTab.svelte` | 42 | 0 | 0 | 352, 353, 365, 378 |
| `ui/src/components/workflows/WorkflowCanvas.svelte` | 6 | 16 | 16 | 1159, 1160, 1168, 1175 |
| `ui/src/components/usage/UsageScreen.svelte` | 28 | 7 | 2 | 326, 329, 332, 334 |
| `web/dock.html` | 8 | 18 | 9 | 47, 49, 112, 117 |
| `ui/src/components/report/Report.svelte` | 20 | 2 | 0 | 205, 206, 207, 209 |
| `ui/src/components/workflows/WorkflowList.svelte` | 3 | 10 | 7 | 155, 156, 163 |
| `ui/src/components/routines/RemindersTab.svelte` | 15 | 1 | 0 | 333, 335, 344, 353 |
| `ui/src/components/KnowMe.svelte` | 13 | 1 | 1 | 214, 215, 215, 215 |
| `ui/src/components/MentionPopover.svelte` | 11 | 2 | 2 | 122, 123, 124, 125 |
| `ui/src/components/catalog/CatalogDrawer.svelte` | 12 | 1 | 1 | 317, 331, 333, 363 |
| `ui/src/components/workflows/HistoryPopover.svelte` | 0 | 7 | 7 |  |
| `ui/src/components/workflows/WorkflowsScreen.svelte` | 4 | 4 | 5 | 473, 536, 537, 545 |
| `ui/src/components/providers/AgentCard.svelte` | 10 | 1 | 1 | 281, 284, 285, 286 |
| `ui/src/components/routines/Composer.svelte` | 11 | 1 | 0 | 201, 203, 207, 208 |
| `web/snap.html` | 1 | 4 | 6 | 16 |
| `ui/src/components/rooms/TeamScreen.svelte` | 0 | 6 | 5 |  |
| `ui/src/components/VoiceChat.svelte` | 6 | 3 | 1 | 60, 60, 62, 63 |
| `ui/src/components/workflows/TriggerFields.svelte` | 0 | 6 | 4 |  |
| `ui/src/components/workflows/AddMenu.svelte` | 0 | 4 | 5 |  |
| `ui/src/components/workflows/WfNote.svelte` | 3 | 4 | 2 | 63, 64, 72 |
| `ui/src/components/SignInCard.svelte` | 8 | 0 | 0 | 42, 43, 51, 62 |
| `ui/src/components/catalog/CatalogCard.svelte` | 5 | 0 | 2 | 47, 75, 121, 126 |
| `ui/src/components/ConsentCard.svelte` | 7 | 0 | 0 | 103, 105, 117, 121 |
| `ui/src/components/workflows/ToolOptions.svelte` | 0 | 7 | 0 |  |
| `web/ui-demo.html` | 1 | 1 | 4 | 53 |
| `ui/src/components/AccountRights.svelte` | 6 | 0 | 0 | 137, 141, 144, 152 |
| `ui/src/components/catalog/DiffView.svelte` | 6 | 0 | 0 | 47, 60, 63, 64 |
| `ui/src/components/chat/ChatDates.svelte` | 5 | 1 | 0 | 190, 212, 216, 217 |
| `ui/src/components/routines/TodoCard.svelte` | 6 | 0 | 0 | 208, 210, 220, 225 |
| `ui/src/components/ScreenControl.svelte` | 6 | 0 | 0 | 16, 16, 16, 19 |
| `ui/src/components/sidebar/NeedsBoard.svelte` | 0 | 2 | 4 |  |
| `ui/src/components/workflows/BotPicker.svelte` | 0 | 3 | 3 |  |
| `ui/src/components/workflows/WfSwitch.svelte` | 4 | 2 | 0 | 32, 37, 46, 47 |
| `ui/src/components/workflows/KindChips.svelte` | 1 | 3 | 1 | 58 |
| `ui/src/components/workflows/OutputFields.svelte` | 0 | 2 | 3 |  |
| `ui/src/components/workflows/ScheduleEditor.svelte` | 0 | 2 | 3 |  |
| `ui/src/components/routines/ChannelChips.svelte` | 3 | 1 | 0 | 40, 47, 55 |
| `ui/src/components/workflows/ContextMenu.svelte` | 0 | 2 | 2 |  |
| `web/pet.html` | 0 | 2 | 1 |  |
| `ui/src/components/LimitRing.svelte` | 1 | 0 | 2 | 49 |
| `ui/src/components/WhatsNew.svelte` | 0 | 2 | 1 |  |
| `ui/src/components/workflows/LinkHint.svelte` | 0 | 1 | 2 |  |
| `ui/src/components/workflows/WfEdge.svelte` | 0 | 2 | 1 |  |
| `ui/src/components/catalog/ConnectFlow.svelte` | 2 | 0 | 0 | 216, 222 |
| `ui/src/components/catalog/YourSkills.svelte` | 2 | 0 | 0 | 65, 97 |
| `ui/src/components/devices/DevicesSettings.svelte` | 0 | 1 | 1 |  |
| `ui/src/components/devices/PairPlaques.svelte` | 0 | 0 | 2 |  |
| `ui/src/components/Dock.svelte` | 0 | 1 | 1 |  |
| `ui/src/components/remote/RemotePcs.svelte` | 0 | 1 | 1 |  |
| `ui/src/components/workflows/ToolArt.svelte` | 2 | 0 | 0 | 36, 43 |
| `ui/src/components/catalog/Catalog.svelte` | 1 | 0 | 0 | 288 |
| `ui/src/components/drawer/DrawerProfile.svelte` | 0 | 0 | 1 |  |
| `ui/src/components/ModelPicker.svelte` | 0 | 1 | 0 |  |
| `ui/src/components/remote/RemoteChat.svelte` | 0 | 1 | 0 |  |
| `ui/src/components/remote/RemoteOrb.svelte` | 0 | 1 | 0 |  |
| `ui/src/components/routines/TodoRecurrence.svelte` | 1 | 0 | 0 | 89 |
| `ui/src/components/sidebar/BotList.svelte` | 1 | 0 | 0 | 456 |
| `ui/src/components/workflows/WfShape.svelte` | 0 | 0 | 1 |  |

## Файлы-исключения (читает проверка кита)

Нарушения в этих файлах — предупреждения. Новый файл сюда не добавлять: он должен собираться из кита. По мере переезда экрана (этап B) его строку убирают.

- `ui/src/components/AccountRights.svelte`
- `ui/src/components/ChatModelPopover.svelte`
- `ui/src/components/ConsentCard.svelte`
- `ui/src/components/Dock.svelte`
- `ui/src/components/KnowMe.svelte`
- `ui/src/components/LimitRing.svelte`
- `ui/src/components/MentionPopover.svelte`
- `ui/src/components/ModelPicker.svelte`
- `ui/src/components/NewBotModel.svelte`
- `ui/src/components/ScreenControl.svelte`
- `ui/src/components/SignInCard.svelte`
- `ui/src/components/UpdatePlaque.svelte`
- `ui/src/components/VoiceChat.svelte`
- `ui/src/components/WhatsNew.svelte`
- `ui/src/components/catalog/Catalog.svelte`
- `ui/src/components/catalog/CatalogCard.svelte`
- `ui/src/components/catalog/CatalogDrawer.svelte`
- `ui/src/components/catalog/ConnectFlow.svelte`
- `ui/src/components/catalog/DiffView.svelte`
- `ui/src/components/catalog/YourSkills.svelte`
- `ui/src/components/chat/ChatDates.svelte`
- `ui/src/components/devices/DevicesSettings.svelte`
- `ui/src/components/devices/PairPlaques.svelte`
- `ui/src/components/drawer/DrawerFiles.svelte`
- `ui/src/components/drawer/DrawerGoals.svelte`
- `ui/src/components/drawer/DrawerProfile.svelte`
- `ui/src/components/drawer/FileViewer.svelte`
- `ui/src/components/orchestrator/Stage.svelte`
- `ui/src/components/providers/AgentCard.svelte`
- `ui/src/components/providers/ProviderList.svelte`
- `ui/src/components/remote/RemoteChat.svelte`
- `ui/src/components/remote/RemoteOrb.svelte`
- `ui/src/components/remote/RemotePcs.svelte`
- `ui/src/components/report/Report.svelte`
- `ui/src/components/rooms/RoomMembers.svelte`
- `ui/src/components/rooms/RoomResources.svelte`
- `ui/src/components/rooms/TeamScreen.svelte`
- `ui/src/components/routines/ChannelChips.svelte`
- `ui/src/components/routines/Composer.svelte`
- `ui/src/components/routines/HabitsTab.svelte`
- `ui/src/components/routines/RemindersTab.svelte`
- `ui/src/components/routines/TodoCard.svelte`
- `ui/src/components/routines/TodoRecurrence.svelte`
- `ui/src/components/routines/TodosTab.svelte`
- `ui/src/components/settings/SettingsHint.svelte`
- `ui/src/components/settings/VaultSettings.svelte`
- `ui/src/components/sidebar/BotList.svelte`
- `ui/src/components/sidebar/InstaboxCard.svelte`
- `ui/src/components/sidebar/NeedsBoard.svelte`
- `ui/src/components/usage/UsageScreen.svelte`
- `ui/src/components/workflows/AddMenu.svelte`
- `ui/src/components/workflows/BotPicker.svelte`
- `ui/src/components/workflows/ContextMenu.svelte`
- `ui/src/components/workflows/HistoryPopover.svelte`
- `ui/src/components/workflows/KindChips.svelte`
- `ui/src/components/workflows/LinkHint.svelte`
- `ui/src/components/workflows/OutputFields.svelte`
- `ui/src/components/workflows/ScheduleEditor.svelte`
- `ui/src/components/workflows/ToolArt.svelte`
- `ui/src/components/workflows/ToolOptions.svelte`
- `ui/src/components/workflows/TriggerFields.svelte`
- `ui/src/components/workflows/WfEdge.svelte`
- `ui/src/components/workflows/WfNode.svelte`
- `ui/src/components/workflows/WfNote.svelte`
- `ui/src/components/workflows/WfShape.svelte`
- `ui/src/components/workflows/WfSwitch.svelte`
- `ui/src/components/workflows/WorkflowCanvas.svelte`
- `ui/src/components/workflows/WorkflowList.svelte`
- `ui/src/components/workflows/WorkflowsScreen.svelte`
- `web/app.js`
- `web/dock.html`
- `web/index.html`
- `web/messengers.js`
- `web/pet.html`
- `web/providers.js`
- `web/screen.js`
- `web/signin.js`
- `web/snap.html`
- `web/style.css`
- `web/turn-card.js`
- `web/ui-demo.html`

## Этап C1 (старые экраны в `web/`) — 17.11.2026

Сделано: кнопки `.primary/.ghost/.danger` в `index.html`, `app.js`, `providers.js`, `signin.js`, `screen.js`, `attach.js`, `updates.js`, `voice.js`, `snap.js` переписаны на `.k-btn k-btn--*`, самодельные скины кнопок в `style.css` (`#drawer`, `#modal-settings`, `.kit-dialog`) удалены; все нативные `<select>` одеты в `.k-select` (`app.js`, блок «Выпадающие списки кита», `window.DeloistKitSelect`), скины `.s-select/.dock-select/.field select` удалены; переключатели `.switch` → `.k-switch-box`, галочки `.quiet-check/.skill-check` → `.k-check`; `input[type=time|number]` → `.k-input`; логотип → `.k-logo`; зелёный/синий/жёлтый в `style.css` и `screen.js` заменены на `--k-brand/--k-attention/--k-danger`; свечение карточек чата снято (остаётся у поля ввода и главной кнопки).
Остаётся: алиасы `.primary/.ghost/.danger` в `kit.css` (их ещё используют Svelte-экраны этапа C2), `.msg-chip/.msg-seg/.tc-stop` и кнопки `snap.html`, `dock.html` (свои классы), кольца в `app.js`/`providers.js`/`team.js` (не значки), `ui-demo.html`, `pet.html`, `#8e6ac9` в `signin.js` (цвет заглушки аватара), `BOT_COLORS` в `app.js`, `voice.js` (свой `<style>` с `--color-*`), около 270 предупреждений `style.css` (размеры и шрифты вне токенов).
