# Changelog

Format — [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), versions — [SemVer](https://semver.org/).

## [Unreleased]

- **File mode ("This computer").** A file on this computer outside the bot's folders (its workspace and project folders)
  is read and written by a bot only inside a folder the person allowed — via the "This computer" button on a one-line
  card. One click allows that one file; the card also offers "in this conversation" and "always allow here", remembered
  per folder and for this bot only. The allowed task resumes on its own, and an answer after a pause continues it exactly
  once. It asks in every mode, including "free rein"; nothing changes inside the bot's own folders. Secrets, keys and
  ssh-config are not opened by the card, as before; real paths are compared against the allowed folder, so a junction or
  symlink pointing outward gains nothing.

- **Linux archives in every release.** The GitHub release gets `Deloist-linux-x86_64.tar.gz` (the app with a window),
  `deloist-server-linux-x86_64.tar.gz` (server without a window) and `SHA256SUMS-linux.txt`; the README links "Linux (beta)".
  The server image of a release is published with its version number and `latest`.

## [0.4.8] — 2026-11-20

Bots on any model work without needless prompts, set up the page watcher themselves, and routines no longer pile up
pauses on a limit.

- **No needless permission prompts on Gemini or any other agent.** The program recognises its own tools by their secret
  name whatever format the agent uses: each format (GPT, Claude, Gemini) used to be added by hand, and on Gemini every bot
  action waited for a card, so unattended routines stalled. Plus one general rule, "what is not forbidden is allowed": an
  agent action with no command and no files asks only if it looks like paying, sending, deleting or publishing, or if the
  bot is in strict mode. Commands, files, keys, releases and the "Always ask" list are checked as before.
- **Routines do not pile up pauses on a limit.** While a model was on a limit, each routine term used to add one more
  paused run, and when the pause ran out they woke one by one every 30 seconds with "The limit has reset". Now a new
  routine term replaces its stuck run, so only one waits.
- **Pause time with a date.** When the run continues on another day, the program says "I will continue at 22.11 16:00".
- **Gemini with two accounts.** The "Gemini" entry offers only its own account's models; the second account's models are in
  "Gemini · <account>". Before, a model of the second account could be picked for the first and fail with "Unknown model ID".
- **The bot sets up the page watcher itself.** Tell a bot "watch for new posts" and it sets the page watcher on its routine
  (`create_routine`/`update_routine` with `watch_pages`), with no card and no addresses or selectors from you: the watcher
  only reads pages, like the bot's browser. Bots learn about it from their help and offer it to routines that check a page
  for something new. `list_routines` shows which routine has a watcher and its last error.
- The page watcher trims tails such as ";" at the end of an address or selector left by copying.
- Linux build: tests that check only Windows rules no longer run on Linux.
- A chat attachment is on disk before the program answers "saved": under load a bot could read an empty file. The run,
  decision and usage journals are written the same way.

## [0.4.7] — 2026-11-19

Routines wake the model only when there is work: page watcher, pre-run check, cron with a time zone.
The Linux server builds again, and Claude bots no longer lose their tools after an update.

### Routines

- **Page watcher.** On its schedule a routine opens pages in the bot's browser itself (for example
  `x.com/notifications` or a profile) and wakes the model only when new matching items appear. With nothing new
  the model is not called and no tokens are spent. The first check only remembers the items; each is handled
  once. Set it up in Settings → Page watcher: routine, addresses, item selector and a filter word.
- **Pre-run check.** A routine can have a check command: exit 0 wakes the bot and passes it the output as data,
  exit 1 means nothing new and the model is not called. Only the human sets it.
- **Cron with a time zone.** A routine schedule can be a five-field cron expression with an IANA zone
  (for example, `Europe/Moscow`); without a zone it uses the computer's time.

### Fixes

- **Claude bots keep their tools after an update.** The name of the Deloist tool set changed on every app start,
  and a continued conversation called the tools by the old name, so the bot decided it had no browser or
  routines. Now the name is one per install. Installing 0.4.7 changes it one last time.
- **The Linux server builds again** (0.4.6 skipped the Linux package).
- Cancelled browser requests no longer pile up in memory; a deleted bot's screen with its last frame is freed;
  the GigaChat uploaded image table is capped.
- A repeat build without changes no longer rebuilds the whole program.

## [0.4.6] — 2026-11-19

Fixes from the Artist and Marketer X logs: tasks reach the other bot, and errors are visible.

- **A card for the human no longer holds the bot.** While a card waited (a new rule, a question), handing a task,
  sharing a file and the browser got "browser action still running" for bots on Codex and Claude, so the Artist's
  pictures and the Marketer's tasks did not arrive. Now only the card itself waits; only browser actions go one at a time.
- **Tool errors of Codex bots are visible.** In the flow of work, Deloist tool steps of Codex showed empty: no
  arguments and no reason for the error. Now you see whom a task went to and why it was refused.
- **Own file by full path.** Browser upload and file tools accept a full path to a file in the bot's own folder
  instead of answering "path outside the workspace".
- **Claude bots see all tools at once.** Claude used to hide Deloist tool descriptions and call them blind, without
  required fields (a click without a number, a routine without a number, a specialist without a name).
- Browser keyboard: one key or one chord per press; the description and the error say so.

## [0.4.5] — 2026-11-19

Teams and a readable flow of work: you see what a bot is doing and whom it handed what; a bot belongs to one team
with a shared folder and browser; routines and tasks repeat like in the Deloist planner.

### Team

- **Team list on one level.** In the sidebar a team's bots sit in one row under it, the lead first with a small
  crown.
- **One bot, one team.** A bot can't be added to a second team. A bot that was in several teams stays, after the
  update, in the one it leads (among equals, the larger one); the others get a line about the move. History search,
  team memory, folders and the roster see only that team. «Bring in a specialist» offers specialists of the own team
  and free ones; undecided cards survive a restart.
- **Shared team folder and browser.** A team bot works in `workspaces/team_<id>`; personal `memory.md`, `brain.md`
  and the program-built `AGENTS.md` and `CLAUDE.md` stay with the bot. A member's files are copied into the shared
  folder once, on joining, without overwriting. Agent instruction files (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md` and
  similar) can't be written there; scripts and autorun files need a card. In the shared folder a bot can change or
  delete teammates' files without a card — it is a shared folder.
- One team browser with shared sign-ins; each bot has its own tabs and can't see or touch another bot's tab. While
  waiting for a card a bot doesn't hold the browser. When the browser owner leaves, the shared browser closes, another
  member becomes the owner, and the leaver keeps its own profile.

### Hand-offs

- **One way to hand work over.** The synchronous `delegate` is gone: a hand-off is always a task (`assign_task`), the
  sender doesn't wait, the result comes back as one update. A task may have a deadline; when it passes, the sender gets
  a line. A short question to a teammate is `ask_teammate`: answered in the same turn, at most two per turn; at most 8
  hand-offs per request of yours.
- Other bots' results arrive as data, not instructions. The sender's rights (mode, «always ask», chain folders) travel
  with the task and survive the queue and a restart.
- **Hand-offs in full view.** The «Hand-offs» window: who handed what to whom, state, deadline, who started it (you,
  a routine, a bot), tokens, result and files; «Cancel» for any open one. «Needs you» shows «Done while you were away».
  A result file is a card with «Open».
- **Bot tasks in the to-do list.** A «Mine / Bots / All» switch; when creating a task — «Assignee: me or a bot». A task
  assigned to a bot goes to it as your message when due, and the bot's result is written into the task. A bot can't use
  the to-do list to assign a task to another bot.

### Chat

- **Human step labels** for every tool, without `mcp__…` and service `ToolSearch`; the step's target (file, site,
  query) next to it, secrets hidden.
- **Short chat.** After a turn its steps fold into one line «Worked 29 s · read 2 files, searched 3×»; the last 6 stay
  live while it runs. A hand-off is one card line for the requester: to whom, state, «Stop», result folded after
  1400 characters.
- **Claude Code helpers are visible.** When a Claude Code bot calls its own subagent, the chat shows «Helper: …» with
  its state (working / in the background / done) and its steps on expand.

### Routines and tasks

- **Repeats like in the Deloist planner** — one rule for routines and tasks: every day, weekdays, every Tuesday, every
  2 weeks, a day of the month, the last day, «3rd Saturday», «last Friday», once a year («3rd Saturday of September»).
- **«Start from a date»** and **«Once»** for routines: a routine doesn't wake before the date; a one-time routine turns
  itself off after running. Bots set these as fields instead of hiding a date in the prompt. Old «time + weekdays»
  routines are converted automatically.

### Rights and files

- **«What the bot may do without asking»** on the bot screen, in plain words: mode, sites, messengers, what is always
  asked.
- **An «Always ask» list** (commands, sending, publishing, payments, deleting, passing files, changing bots): works in
  any mode and along the whole hand-off chain, is not overridden by «always allow» rules; only you can change it.
- **One attachment layout:** copies from other bots go to `attachments/<date>/from-<bot>/` and are removed after
  30 days; your own attachments are never removed.

### Mail

- **Mail as a bot channel.** IMAP/SMTP mailboxes (Yandex, Mail.ru, Gmail, your own server); the app password is kept
  in secrets and never shown to the bot. A letter becomes a task only from a trusted sender and only with a DKIM/DMARC
  signature verified by your mail server; other letters are data. Sending without a card — only to allow-listed
  addresses and at most 20 an hour; a new address gets a card with the full text. A routine mail trigger wakes the model
  only when a matching letter arrives.

## [0.4.4] — 2026-11-18

Tasks now get finished: restarting the app, a model limit, a switched-off computer or a silent human
no longer cut the work short.

### Added

- **Resume after a restart.** Every run — from your message, a routine, a goal, a team task, a webhook —
  is saved before it starts, and steps land in the chat as the work goes. After a restart, an interrupted run is resumed
  once on its own with a note "you were interrupted at step X". If the interruption hit while sending, publishing, paying or running an unfamiliar
  command, the bot repeats nothing by itself: the run waits in "Needs you" with the buttons "Resume" and "Cancel the run".
- **A limit means a pause until reset, not a failure.** When a subscription or key limit runs out, the bot writes "I'll continue at 14:00" and
  continues on its own, even after a restart. A fallback model is tried first. Unattended work
  pauses already at 95% of the subscription window, a live conversation does not. A daily or monthly token cap pauses until the reset.
  One notification "N bots are waiting", and a "Resume now" button.
- **A bot can wait:** "I'll continue in 30 minutes / at 14:00 / when the webhook arrives" — with no process left hanging.
- **A clear result for unattended work.** A routine, goal or team task ends with a result: "done / continuing /
  needs you / couldn't / nothing to report", plus a note for the next run; a bot that drops the work halfway
  gets one reminder. Watchdogs: three identical replies in a row and 20 minutes of agent silence stop the run.
- **A missed run is caught up** according to the routine's rule: "run the latest one" (up to 12 hours late) or
  "skip"; other missed runs get a single line in the log. The computer does not go to sleep for an hour before a run.

### Changed

- **A permission card no longer refuses after 5 minutes.** With no answer, the run stops and waits in "Needs you",
  the card survives a restart; a later "allow" resumes the run and lets exactly that action through once.
  After 2 hours — one reminder.
- **Every stop comes with a reason:** network, sign-in needed, check failed, stuck, the bot was busy. A network failure before the
  first change is retried automatically after 1, 5 and 15 minutes; after changes — a "Retry" button. A routine no longer
  gives up after a day: after errors it runs less often, and after three unrecoverable ones in a row it pauses with a "Resume" button
  and a single message. A routine waits for a busy bot for no more than an hour, and a run lasts no more than 3 hours
  (a goal — 12).
- The message queue after a restart is no longer paused; team tasks are no longer lost after 6 hours.

### Fixed

- A bot's Chrome left open from a previous run no longer breaks routines with the error "DevToolsActivePort did not
  appear": the app connects to its own window.
- `deloist run` and MCP no longer show "Sign-in needed…" or a pause in place of a successful reply.

## [0.4.3] — 2026-11-17

### Added

- **Bot memory as plain files.** The bot's folder holds two files: `brain.md` — the rules you approved, and
  `memory.md` — everything the bot remembered on its own. Both open and can be edited in the bot's tab as plain text, and go into
  each of its runs. Earlier personal facts and `MAIN.md` were moved over automatically.
- **Reply right from a Windows notification.** A bot's question with options and a "allow once / deny" request
  are closed with buttons in the notification, without opening the chat. Dangerous actions from a notification can only be opened or
  declined.
- **The app icon is Lava's fireball.**

### Changed

- **Limits show real numbers.** A bot's ring on a subscription takes the remainder from the subscription itself, rather than guessing from the
  error text. The reset is shown in plain words ("resets in 3 h") and with a mark of how fresh the data is; after a reset the old
  number no longer lingers. The daily routine cap is visible only when a routine has really stopped, and a bot's monthly cap is in the ring with
  the reset date.
- The orchestrator's summary after "Create" arrives in the team chat.
- Memory review no longer spends the daily routine limit.

### Fixed

- If `brain.md` is edited behind the app's back, the bot notices and shows a card; the bot cannot write `memory.md` and `brain.md`
  with an ordinary file write.
- An agent that does not report limits is no longer restarted every 30 minutes just to check them.
- The open build no longer has the "Devices" and "Other PCs" screens that led nowhere.
- Removed old memory paths, the "one step lower" hint (the model below was always the same) and disabled code.

## [0.4.2] — 2026-11-17

### Added

- **New memory for bots and teams.** Each team has its own memory: the team's agreements and knowledge are seen only by its
  members, while the shared memory is for things about you personally.
- **Rules go into instructions, not memory.** Tell a bot "always…" or "add to the rules" — it shows a card
  with a one-line rule, and after "Apply" the rule is in each of its runs.
- **A `MAIN.md` file in the bot's folder** is always in front of it: current tasks, criteria and where the working files are. In the
  "Files" tab it comes first.
- **A bot finds past conversations** — its own and its teams' — even after an app restart or history compaction.
- **You can see what the bot remembered and where:** a line in the run with the name of the fact, the place and a "Cancel" button.
- **The orchestrator knows its team** — who is in it and what they are doing — and once a day reviews the team's memory: merges duplicates,
  fills in what is empty, suggests rules.
- **"Always in the run"** for a memory fact: a pinned fact goes into every run in full, the rest as a single line and
  picked by topic. Personal memory in a run became about two thirds shorter.

### Changed

- Memory review after a task works even without a Deloist balance: through a connected model or a quiet run of the bot itself.
  If the review fails, the bot says so in the chat once a day.
- The bot records facts about you only with your verbatim quote. Neither the bot nor the memory doctor changes your entries, and
  what you deleted is not learned again.

### Fixed

- After an app restart the bot continues the previous conversation instead of starting with a short recap.
- Saving to memory no longer loses details and no longer waits for a browser action to finish.
- A bot script holding the app's key can no longer edit or roll back memory, change the profile or launch the doctor.

## [0.4.1] — 2026-11-17

### Added

- **A new look, "Embers".** A warm orange accent instead of blue, glossy buttons with a thin edge, a soft glow
  only on the active element. Three colors: orange, a warm neutral and red — only for errors and deletion.
- **Our own dropdown list and menu** with search and groups instead of system ones — identical throughout the app.
- **Smooth motion:** the selection slides over, windows fade in; the Windows "reduce motion" setting is respected.
- **The dock and limit rings** in the new style: dark coal at the screen edge, the limit as a lava arc.

### Changed

- Chat, the sidebar, workflows, "Tasks and automation", the catalog and settings are built from one set of
  elements: buttons, fields and lists are the same size and look everywhere.
- Bots append data rows to their own files and run safe read-only commands without extra cards.
- A bot's cloud browser goes to sleep after 5 minutes of idleness.

### Fixed

- Routine cards are no longer rejected with the error "confirmation mode changed".
- The update waits until the old copy of the app closes, and only then deletes it.

## [0.4.0] — 2026-11-17

### Added

- **Bots don't depend on a switched-on computer.** A free Instabox server in a couple of clicks: the card
  "Connect a free server" in the sidebar and the "Cloud" screen in settings. Bots reply in Telegram
  even when the computer is off.
- **Device linking through deloist.ru.** A phone and another computer connect to the app with a pairing code:
  the "Devices" screen, the "allow device" banner, sections for other PCs in the sidebar and a chat with their bots.
  Everything is encrypted on the devices, and actions from another device require your confirmation by default.
- **Server mode** for an old laptop: the app starts without a window and keeps the system from sleeping.
- **Team orchestrator.** On first launch the main bot asks what to take off your plate, looks for ready-made
  bots in the catalog and assembles a team right in front of you.
- **Team screen** with a diagram of members and a picture the app draws for each new team.
- **A new bot settings panel:** kit-based cards, everything saves itself, permissions in plain words.
- **A file viewer** in the bot panel: code highlighting, a comment on a selection, "Open in VS Code".
  A file can be dragged from the panel straight into the chat.
- **A bot menu in the sidebar** (button and right click): settings, moving to another team. A bot can be
  dragged into a team with the mouse.
- **Auto-update.** A new version downloads and installs in the background, bots are not interrupted. A "Deloist X is ready" banner with a "Restart" button appears at the bottom of the sidebar; without it the new version starts the next time you open the app. It can be turned off in settings ("Update automatically").
- **The "What's new" window.** After an update and via the "What's new" link on the banner: a page for each category ("New", "Improved", "Fixed"), flip with the arrows, live orbs on top. The "All changes" link opens the whole log.

### Changed

- Routines repeat until they succeed; helper reports show an orb, files and a link to the chat.
- Team tasks continue after an app restart instead of being interrupted.
- When you switch models, the app offers to compress the history that the new model would otherwise reread in full.
- A bot learns from its own experience: it sees tool steps and fixes outdated memory entries.
- Bot orbs in a team show the same state as in the header; the search icon is the service from the query.
- All icons are in one Lucide style, and scrollbars are thin and sit inside rounded windows.
- Every bot and team has its own unsent draft.

### Fixed

- The chat feed keeps time order, with no double confirmation; the progress of a run no longer disappears when you switch bots.
- Read-only commands sent by an agent as a single header no longer ask for confirmation.
- "Copy link" on a file copies the path.
- **"What's new" finally opened after an update.** The window crashed on an undeclared variable and silently failed to show.

## [0.3.7] — 2026-11-16

### Added

- **Workflows.** A new section with a canvas: steps with a model, a condition, a picture and files, started by a webhook or a
  message in a messenger, with the result going to a file or the chat. Workflow versions, an edit history, and bots can call workflows.
- **Tasks and automation.** Routines moved to the panel on the right: each is one line with its next and previous run.
  Routines and tasks created by a bot show up as cards right in its chat.
- **Dates and +Name in the input box.** Dates are highlighted and turn into tags, and "+Name" assigns a task to a bot — in the chat and in the planner.
- **A free start.** One-click sign-in to OpenRouter, with a free router by default.
- **Your own MCP servers with sign-in.** OAuth sign-in is supported: a "sign-in needed" state and a field for your own client ID.
- **Live glass orbs.** 17 looks were redrawn, Lava has flowing lava and glowing eyes; calm orbs don't use resources.
- The "Needs your attention" board is split into four sections, and the queue shows the reason for a pause.

### Changed

- An agent has its own session in each team: the context of one team does not pass into another.
- Assignments outside a team go only between leads. Agents update next to the installation and check the registry once an hour.

### Fixed

- **`deloist team apply` works again.** The app's key cannot add a team directly (bot scripts can read it), so the command line submits a request, and a person accepts it with the "Add team from file" card in the window. The flags are `--wait SEC` and `--no-wait`, and `--json` returns `status` and `imported`. The card shows a verification code, the beginning of each persona, skill and routine, and warns about long texts and large teams; there is a "Decline all and don't accept requests for 1 hour" button, and a declined file is not accepted for an hour. Request events are seen only by the window, and status and revocation are read through a closed endpoint.

## [0.3.6] — 2026-11-16

### Added

- **A team proposed by a bot.** A bot picks the team's makeup, looks for ready-made bots in the catalog and shows a card
  with full instructions, access and settings; the app's folders are highlighted in it. Nothing is created without your "yes".
- **Team work without waiting.** The lead hands out tasks to members, watches their progress and shares files; when a member's task
  is closed, a short card line appears in the lead's chat.
- **Member suggestions as cards.** A member can suggest changing a bot or setting up a routine — you decide.
  Cards waiting for an answer don't disappear after the window reloads.
- Agents connected through MCP can ask you a question right in the chat.
- **Models and connections in settings.** A separate section with providers and accounts; available agent updates appear as a notification with an install button.
- **A compact browser screen.** A preview at the top right, a "Take control" button inside the screen, expand and a separate window.
- **File format icons.** Images, audio, video, documents, PDF, spreadsheets, presentations, archives and code are told apart in the bot's drawer.
- **A live "Mechanism".** The large avatar is animated; thumbnails and other looks stay static.

### Changed

- Sidebar: "All bots" are separated from real teams, search opens with an icon or Ctrl+K, and settings are gathered in a menu. Limit rings are always visible.
- The model list loads in advance and stays available during a background refresh. The X icon on links became recognizable.
- The queue explains a manual pause and recovery after a restart; a manual stop is kept after a restart.
- **"Conversation memory" is a slider.** Instead of three modes, a bot's profile has a slider "Compress at N% of the limit"
  (20–95%): when the history grows to that share of the model's window, the old part is folded into a summary.
  Under the slider you can see the number of tokens for the bot's model, and a price warning for a large window.
  For bots that never had the slider set, everything works as before. The Gemini 4 model window was added to the table.
- All model settings are gathered on the "Models and connections" screen.
- Teams in the sidebar update immediately and show members and a preview without markup.

### Fixed

- Avatars recognize ACP browser actions, show supported sites and clear the wait for a human after work resumes.
- Clearly unfinished ACP work gets a limited auto-continue within the budget; Stop, a refusal and lack of progress halt it.
- A timeout of a separate connection check does not switch off a working model. Gemini quota read errors are told apart from missing limit support; a changed token file refreshes the cache.
- Checks for writes to autostart and Git files, hard links, passing permissions between bots and direct reading of secrets through ACP were tightened. Repeated empty confirmations from the built-in browser were removed.
- Fixed the rendering of bot list tooltips; the API driver's User-Agent contains the current app version.
- Work log: in the collapsed view the bot's text remains, steps have short captions, and files have previews.
- The message footer shows the correct duration, links to local files open, and long lines don't cause horizontal scrolling.
- Creating and deleting bots and rooms, writing routines and importing a team are available only to the app itself, not to bot scripts.
  The agent bridge went through two more rounds of security review: bypasses via a stream and short file names, a race
  for the shared minimum of permissions and planted executable files were closed.
- A task put in the queue keeps the limits of whoever assigned it; file exchange in a team follows the write rules.

## [0.3.5] — 2026-11-14

A planner inside "Routines": tasks, reminders and habits. Consent to documents, key encryption and a shared card for all agents.

### Added

- **Tasks in "Routines".** Sections "Main / Current / Someday", a due date, a checklist, repeats, moving overdue tasks
  to today, auto-archive and an archive with "Restore", and search. A task can be written as a phrase: "call mom tomorrow at 10:00",
  "take out the trash every Wednesday". Dates, times and repeats are parsed by code, without a model; several chores in one phrase
  are split by your model, and if you don't have one, the task is still created. Bots see the list, add and mark tasks.
- **Reminders without a model.** A reminder routine fires by the clock and spends nothing: at the time of the event or
  5, 15 or 60 minutes before, as a Windows notification or in Telegram. If the computer was off for more than 12 hours,
  the reminder is marked as missed. Notifications are signed "Deloist".
- **Habits.** A "done today" mark, a streak of days, "N of M this week", templates, a reminder on your days.
- **An input box with date highlighting.** A date, time and repeat are highlighted right in the text and shown as chips.
- **An agent card for everyone.** Claude, ChatGPT, Grok and other agents got a card like Gemini's: the version
  and "Update", "Restart", models with switches, "Default for new bots".
- **Agents from npm without Node.** Deloist downloads the official Node LTS itself (verifying SHASUMS256) if it is missing.
- **Split view.** A second bot or group side by side or one under the other.
- **The "Team" panel.** A live tree: who the lead bot handed work to and what the helpers are doing.
- **Chat table of contents.** A list of sections to get back to any part of the conversation.
- **Grok limits.** The weekly window is shown as a circle, like the others.
- **Moli browser for bots** — one more browser engine to choose from.
- **Consent to documents.** Separate checkboxes for personal data processing and for the offer; in the account window —
  a refund, an export of your data and account deletion in two steps.

### Changed

- **Keys are encrypted** with Windows tools (DPAPI, your account only). You can go back to 0.3.4 by renaming
  `secrets.json.v1` to `secrets.json`. If `secrets.json` can't be read, keys are not overwritten: writing is blocked,
  and the window shows "keys unavailable" and a "Start with empty keys" button.
- **The chat input box** is a single card with a mode strip; messenger permissions moved to the messenger card.
- **"Get acquainted" opens faster:** the source list takes 0.7 s instead of 2.5 s, and instantly on repeat.
- **A cloud version is in preparation:** the app builds for Linux and encrypts keys there too. The Linux release comes later.

### Security

- The app's key (a bot script can read it) can no longer sign out of the account, change the plan, top up
  the balance, sign in, give consent, request a refund, export data or delete the account.
- The companion files `secrets.json.*` are closed to bots the same way as `secrets.json` itself.
- Data export does not overwrite someone else's file in "Downloads"; links to documents go strictly to the Deloist site.

## [0.3.4] — 2026-11-13

One-click connection, messengers, a bot's sign-in to sites through a password manager, and a new window look.

### Added

- **Gemini in one click.** The "Connect" button downloads Google's program (Antigravity CLI) by itself with signature
  verification and opens Google sign-in. With a Google subscription, Gemini, Claude and GPT-OSS are available. Up to three accounts,
  each bot has its own; instead of signing in you can specify an API key.
- **Messengers.** Your own bot in Telegram, Discord and Slack: pairing by code, replies back to the chat, screens in settings.
  Rules for messengers in the bot's browser: what can be read and sent without asking.
- **Bitwarden and 1Password password managers.** A bot signs in to sites by itself: the app inserts the password and code, and the model
  doesn't see them. The first sign-in to each site goes through your card; without you — only where you allowed "always".
- **"I'll bring in a specialist".** A senior bot sees the team's specialists and suggests a suitable one with a card
  "Hand over" / "I'll handle it myself".
- **Economical task handoff.** A specialist gets a short brief without others' history and returns a summary of up to 20 lines;
  the chat shows how many tokens the handoff cost.
- **Catalog: 32 programming and game engine specialists** — Next.js, Vue, Angular, Kotlin, Swift, Flutter,
  Java/Spring, C#/.NET, C++, PHP/Laravel, Ruby/Rails, Python for data, DevOps, Godot, Unity, Unreal; versions
  checked against the official sites.
- **A conversation branch:** "Branch from here" under a message makes a copy of the bot with the history up to that point.
- **Groups:** a group archive and "Hand over…" to another member.
- **Routines to the calendar:** an "Add to calendar" button (.ics).
- **Import from Hermes** in "Get acquainted faster".
- **Limit circles** for each connected provider: week and 5 hours, with percentages and reset time on hover.
- When the window is 85% full, ACP agents start a new session with a summary.

### Changed

- **A new window look** in the Arc style: graphite "islands", a collapsible panel (Ctrl+B), search (Ctrl+K), the input box
  as a card, soft animations.
- **Model picker** — a strip of provider logos on the left, search across all models, compact rows.
- The drawer tabs "Memory", "Files", "Goals" are on the new kit.
- `deloist.exe` is a single file: `WebView2Loader.dll` is no longer needed next to it.
- The chat opens at the bottom right away when you switch bots.

### Fixed

- A `.env` file given by its full path was read without a "secret" card.
- The context window of GPT-6.1 Terra through Codex was shown as 400 thousand instead of 1,050,000.
- The provider list in the "Connect your own AI" window was sometimes empty.

## [0.3.3] — 2026-11-12

The interface moved to the new design kit: chat, sidebar, the bot drawer and all windows in one style.

### Added

- **A new sidebar.** Pinned bots on top, then "Team", and free specialists collapsed into
  "Specialists in reserve". To pin a bot, use the pin that appears when you hover a row.
- **Guests in chat.** When a bot hands a task to a specialist, the specialist appears in the chat header as a guest: click the name to open
  their chat, and the cross sends them back to reserve.
- **Bot profile in sections:** appearance, who it is, model, permissions, skills; "Save" is always at hand.
- **Chat on the kit:** a permission card, tables and site icons next to links in replies, collapsed repeated steps and
  a step feed in order.
- **Model and effort in one click** from the chat header; for ACP agents you can pick the effort and their extra settings.
- **Web search without a browser:** ChatGPT (Codex) uses its own search, and bots on an API get the
  `web_search` tool.
- **The bot's browser window** opens by itself on sign-in pages and at the bot's request; it can be stretched and minimized.
- **Specialists in the catalog:** Go, Python, TypeScript/Node, Svelte, PostgreSQL, React, Godot 4, Unity 6.
- ACP agents got team tools (hand over a task, list bots, remember, recall) and routines.
- The windows "Connect your own AI", "New bot", "New group", "Routines", "Shared memory", "Feedback", "Account" and
  settings in six sections are on the new kit.

### Changed

- A bot browses sites more economically: after a click it gets a short snapshot of the page, not a full one.
- The "Join", "Subscribe" and "Follow" buttons on sites now ask permission with a card.

### Fixed

- "Conversation memory" in the bot profile is now saved.
- Saving settings no longer resets the model if the agent list hasn't loaded yet.
- The check for a risky click looks at the button, not at the message text next to it.
- The bot's browser no longer offers "Restore pages?" after the app is closed.
- Selected text is visible in the dark theme.
- Choosing a model no longer writes an error to the log when the app starts.

## [0.3.3-beta.1] — 2026-11-12

Beta: the first screens on the new design kit, a new model picker, an icon and an installer. The remaining screens will move over in 0.3.3.

### Added

- **The app icon** in `deloist.exe`: the shortcut, Explorer and the taskbar no longer show an empty file.
- **A new model picker.** Provider logos on the left, models with search on the right; the list holds only what you have
  connected. The "+" button and the "Connect a new AI…" line open the connection window. The `default` alias for agents
  is labeled "Default — the agent chooses". Logos are lobe-icons (MIT).
- **A two-column settings window:** sections "Model", "Appearance", "Skills and connections", "Security",
  "Browser", "Night shift", "Updates", "Advanced".
- **Buttons under a bot's reply:** "Copy", "Read aloud" and the reply time — in the main chat and in the dock chat.
- **Model tiers for bots:** strong / regular / fast, and effort. After two failed runs in a row the bot takes
  a model one tier stronger, and after a success it goes back.
- **Specialist catalog:** a new specialist-bot schema and the first bot, "Rust developer".
- **The `Deloist-Setup.exe` installer** (about 0.6 MB). Download it, run it — it downloads the app by itself, verifies the
  release key signature and checksum, installs to `%LOCALAPPDATA%\Programs\Deloist` (no administrator rights),
  creates a Start menu shortcut and, if you agree, a desktop one, adds an entry to Windows "Apps" and, if
  needed, installs Microsoft Edge WebView2. Uninstalling asks whether to keep your data. Auto-update works as
  before: it replaces the same `deloist.exe`. The old release files (`deloist.exe`, `WebView2Loader.dll`) stay.

### Fixed

- An agent that reports "Not logged in" itself (as Claude does without a sign-in) is no longer counted as connected:
  the connection window offers to sign in, and it is absent from the model picker.
- The chat header shows the bot's own model, not the model from settings.
- "Get acquainted faster" shows sources at once: previously the app read the whole Claude Code history just for a counter,
  and the window stood empty for minutes.
- The chat in the dock is no longer cut off on the left.
- An update from the beta to the release of the same version is now offered.

## [0.3.2] — 2026-11-12

Voice conversation with a bot, free models that a bot connects by itself, and work in your Chrome.

### Added

- **Voice conversation.** The button next to the microphone opens a hands-free conversation: you speak — the bot hears the end of your phrase,
  replies briefly and starts speaking from the first ready phrase. You can interrupt, and the bot goes quiet and listens.
  If a consent card is needed, the conversation pauses until you answer. The voices are built into Windows.
- **Free models.** Ready-made providers with free plans: NVIDIA NIM, Groq, Google AI Studio, OpenRouter
  (`:free` models at the top of the list) and Z.ai (GLM Flash). A "Free" badge, a "Where to get a key" link and a step-by-step
  hint. The skill "Connect free models": the bot opens the key page itself, and the app saves the key —
  the bot never sees the key, and the card shows only the last 4 characters.
- **"Sign in to Google in the bot's browser".** You sign in to Google once in the bot's browser (only you type the password),
  and the bot can then sign in to sites through Google. There is a card in the first chat and a button in the bot's settings.
- **"Work in my Chrome"** (advanced, off by default). The bot acts in your regular Chrome through an
  extension. The extension links to the app by a code, you pick the allowed sites in the extension itself,
  and banks, Google, Microsoft and Yandex accounts, password managers and Gosuslugi are always blocked. It is turned on
  for an individual bot.
- **Conversation window by model.** Before, all bots had one window — 120 thousand characters (about 40 thousand tokens).
  Now the app knows the model's window (Claude, Gemini, DeepSeek, the Deloist subscription, GigaChat, Alice and YandexGPT,
  and for the rest it takes it from the provider's model list; for Ollama — what the server will actually load).
  The bot profile has a new "Conversation memory": "Regular" (as before, the default), "Large (for
  programming)" — half of the model's window, and "Model maximum" — 80%. The context indicator under the input box
  shows tokens against the model's window. External agents (Claude Code, OpenCode and other ACP) manage context themselves.
- **Mentioning bots in chat.** Type `@` and a list of your bots appears. A bot can assign a task to a mentioned bot
  without an extra card.
- **"Get acquainted faster".** With your permission the app reads your messages in Claude Code, Codex, OpenCode
  and Gemini CLI, strips out keys, passwords, emails and phone numbers and suggests a profile and personal bots. Before sending you can see
  which model the topics will go to and whether it is a cloud one; you edit the profile yourself before saving.
- **Version and "What's new".** The version is visible in settings, and after an update a window with the changes opens.
- **Ollama Cloud** — in the provider list (paid, by tokens or subscription).

### Fixed

- A bot script holding the app's key can no longer change settings and providers (for example, redirect your key
  to its own address or swap an agent's command).
- Encoded PowerShell commands (`-EncodedCommand`) are checked the same way as regular ones: secrets can't be read
  through them.
- External agents via MCP can save facts to memory again.

## [0.3.1] — 2026-11-12

The first version for everyone: a catalog of ready-made bots and skills, free voice on your own computer, and a new dock.

### Added

- **Catalog.** "Skills", "Agents" and "Apps" appeared in the sidebar: 20 ready-made skills and 8 agents
  (secretary, researcher, editor and others) install with one button. Before installing you can see what the bot
  will get and where the key will go; your own edits to a skill are not lost on update, and you can remove it at any time.
- **Free voice.** Speech recognition runs on your computer: the model is downloaded once,
  and after that neither the internet nor a balance is needed. Paid cloud voice is hidden in this version.
- **Hotkeys.** Voice to the bot and "Show on screen" (circle an area — the snapshot goes to the bot along with your question),
  and also editing selected text with a key combination. They are set up in the main window.
- **A new dock.** Limit rings, a tray with a gear, a microphone next to the bot's avatar: you can see how recording,
  model download and recognition are going. When a bot reads a reply aloud, its ring "sounds".
- **Model choice in one click** right in the bot profile.
- **Connecting apps.** Keys are stored encrypted, you sign in through the service's website yourself, and before
  connecting the app shows a consent: which commands will run and which sites the data will go to.
- **Keys for projects.** A bot does not read key files and SSH keys without your "Allow" in any mode;
  a secret can be put into commands by name without showing it to the bot.
- **Deloist development mode.** In settings you can open the app's own source code to a bot so it can help
  with development; only you turn it on.

### Changed

- The interface is gradually moving to a new foundation (Svelte): screens that weren't touched look as before.

### Fixed

- The bot's browser starts without an automation flag, and sites like Ozon stopped treating it as a robot.
- Edge as the bot's browser is no longer considered crashed at startup.
- API-token-style keys are masked in replies and in the bot's memory.

## [0.3.0] — 2026-11-11

### Added

- **Memory is a folder of notes.** Everything the bots know is stored as plain Markdown files: the folder can be opened
  in Obsidian, read and edited by hand, and the app will pick up the edit. Word search is fast and works offline.
- **Memory before every message.** The app picks facts and past tasks related to your message by itself
  and puts them next to it in a "From memory" block — the bot doesn't have to recall by itself. What was already shown
  is not served a second time, and replies don't get pricier: the start of the request doesn't change between runs.
- **Episodes.** When a task is finished, the bot writes a short story: what was done, how it turned out, what
  is left. Such episodes then surface when a similar task comes again.
- **A person profile.** Who you are, how to address you and how to talk to you — as a separate note that
  all bots see, in personal chats and in groups. The profile can be edited by hand.
- **Memory fills itself.** After a task, on any model, the app analyzes the conversation and writes down
  what's new; in the chat there is a quiet line "Remembered: …" with a "Cancel" button. People's contacts are first asked about
  with a "Remember contact?" card. Passwords, keys and card numbers never get into memory.
- **Memory for external agents.** Agents that work through their own program get memory as an `AGENTS.md` file
  in their folder and memory tools through MCP.
- **Files and images in chat.** A paperclip by the input box, drag and drop and pasting a screenshot from the clipboard: up to 10 files
  of 10 MB per message, in a personal chat and in a group, and in the queue too. All models see images through the API —
  including GigaChat and Alice; a model that doesn't accept images gets the file path and keeps working.
- **The bot screen.** The "Browser" button opens a live picture of the bot's browser in the app and a line "what it is
  doing now". "Take control" — the bot waits, and you click and type right in the picture; "Give back to
  the bot" — it continues. When the bot calls for help (sign-in, captcha), the screen opens by itself; the real browser
  window remains a fallback. The bot's commands are visible on the same screen as a live terminal with a "Stop
  command" button: only the command stops, not the whole run.
- **"Teach the bot".** Press "Record" and do a task yourself on the bot's screen: the bot records the steps, and passwords are not
  recorded. On "Stop" it writes a skill from the recording — check it, edit it, save it and run "Test the skill".
- **Four answers to permissions.** "Allow once", "Allow in this conversation", "Always allow
  here", "Deny". "Here" means the same folder, site, command or server; payment, sending to people,
  deletion and publishing are remembered separately, and a regular "always" does not open them. Deleting a routine is asked
  again every time.
- **Permission rules.** "Always" answers are gathered in settings under "Permission rules" and removed with the
  "Remove" button; conversation permissions are shown as a line above the input box and switched off there. At night and in routines
  only rules work, conversation permissions do not. While you hold the bot's screen, no rule
  lets its action through.
- **Decision log.** In the bot profile — what it did by a rule or by your answer, in words:
  "clicked 'Export' on the site — allowed by the 'always here' rule".
- **"Needs you".** A button with a number in the sidebar: all permission cards, requests for help in the browser,
  bot questions and memory cards across all bots and groups in one list. A click opens the right chat at the card.
- **Questions with options and forms.** A bot can ask with option buttons or a simple form (text,
  number, date, choice). The question waits for an answer both after the run ends and after an app restart — a late
  answer reaches the bot as a separate message.
- **Routines, goals and self-checks from chat.** Tell a bot "send me a summary every morning" — it sets up,
  changes or deletes the routine itself. Say what to achieve — it sets a goal, tracks progress on it and
  launches steps; goals are visible on the bot's "Goals" tab. Self-check: the bot reviews its own goals and work by itself
  and writes only if it has something to say; anything that changes something, it asks about in such runs.

## [0.2.12] — 2026-11-10

### Added

- **A session line under the input box.** Runs and steps, reply speed (tok/s), how many tokens were spent and
  what share of the input came from the cache, and a small ring showing how full the window is before history compression. On hover —
  details: model and tool time, average time to first token, cached and regular input,
  output. Counters accumulate from the moment the chat was cleared.

### Fixed

- **The "Stop" button is visible the whole time a bot is working,** in groups too, not only when the input is empty;
  Esc in the input box stops it as well. Text typed in the meantime goes into the queue.

## [0.2.11] — 2026-11-10

### Added

- Bots with API models can view the team, create colleagues and change their name, role, instruction, model and color. The calling bot's confirmation mode is respected; access is changed by a person.
- Intermediate text and actions are gathered in a collapsed "Work progress" in personal and group chats. The final reply and permission requests stay visible.
- Temporary API and network errors are retried automatically with a growing pause, without repeating finished tools; the wait is interrupted with the "Stop" button.

### Fixed

- Commands run for up to 30 minutes by default, with a configurable limit of up to two hours; on a timeout the bot gets the error and the output and can continue. "Stop" terminates the whole command tree.
- Sessions of different copies of the app don't overwrite each other. When chat loading fails, the content saved in the window no longer disappears silently.
- Old update responses no longer overwrite a bot's fresh look in the dock and on the pet.

## [0.2.10] — 2026-11-10

### Added

- **History compression.** What falls out of the model's window is folded into a summary rather than forgotten: the model
  sees it at the start of the conversation, and the person sees a divider "Earlier: … folded into a summary". The window is also compressed by
  size, and if the provider answered "context overflow", the run compresses the history and repeats the request. A
  "Compress history" button in a bot's and a group's chat.
- **Memory is a graph of facts.** A fact has a number, a type (about a person, a correction, affairs and decisions, links and accounts),
  a title, details, entities and links. Shared memory is seen by all bots: every request has its table of contents,
  one line per fact, and `recall` opens a fact by number or finds it by words together with related ones.
  A bot sees its personal facts in full. Limits: shared memory 20,000 characters, table of contents 3,000, a bot's own 8,000.
  The "Shared memory" screen and the bot's memory tab: facts by type, editing, moving between memories.
- **Memory doctor.** Once a day on the app's first launch, on overflow and on a button, it removes duplicates
  and outdated entries, merges similar ones, links facts, moves them between shared memory and bots, and turns proven
  ways of working into skills. Every edit — by a bot, a person or the doctor — is in the log and can be undone.
- **Skills learn by themselves.** The bot was corrected, it found a working way, or a skill let it down — it saves a skill
  (`save_skill`), and the chat shows "Skill created…" with a "Cancel" button. Modes in settings: "Automatic",
  "Ask first", "Off". When there are many skills, the prompt holds only their names, and the body comes through `read_skill`.
- **Workspace file graph.** The `workspace_graph` tool: files, document sections and declarations in code,
  linked by references, imports, mentions and uses. A bot finds what it needs by words, sees who
  refers to what, with file and line, and how two parts are connected, without reading the whole folder.
- **Meeting the bot.** In an empty chat the bot says hello and suggests three tasks for its role.
- **The "Screen" bot look.** A round LED screen in a thin frame of the bot's color: a face made of dots, with dot size as
  brightness. It follows the cursor, blinks, listens while you type, works, rejoices at the end of a run, is sad about an
  error, and dozes off after two minutes of silence; emotions change in a wave. Only the color is chosen in the profile.
  A still face doesn't use the processor: a frame is computed only when something changes.

### Changed

- Bots' old memory (lines in `memory/*.md`) moves into facts on first launch, and the files stay with
  a `.old` suffix.

## [0.2.9] — 2026-11-10

### Added

- **The bot dock at the screen edge.** A narrow strip sticks to a screen wall: bot avatars and a "+" for a new one.
  A handle at the bottom moves the dock along the edge, and if you pull it away from the wall, the dock detaches, can be moved
  and sticks to the nearest wall. Clicks pass through the dock to the programs beneath it.
- **A mini-chat from the dock.** A click on an avatar opens a conversation window next to the dock: it grows
  out of the bot and folds back into it when you switch to another program.
- **What the bot is doing.** While a bot works, a strip slides out of the dock by its avatar: the previous step on top,
  the current one below ("Reading report.docx", "Thinking", "Waiting for you").
- **Finished work.** The bot's reply briefly appears as a card next to the dock, and a dot stays on the avatar.
- **"Bot dock" settings:** show or not, above other apps or as a regular window,
  the display (if it is disconnected, the dock moves to the Deloist window), right or left edge, hide behind the edge
  when not needed, and which bots to show.

## [0.2.8] — 2026-11-10

### Changed

- **Freedom in its own folder.** A bot no longer asks permission for every command: in its own folder
  it does what's needed, and it can read anywhere on the computer (`dir`, `type`, files by full path).
  A card appears only when it changes something outside its folder. Keys, browser passwords,
  `.env` and the app's service files are closed in every mode.
- **A switch under the input box:** "Ask permissions" or "Freedom of action" for the current bot.

## [0.2.7] — 2026-11-09

### Added

- **GigaChat and "Alice".** In "Connect your own AI" right after ChatGPT and Claude: GigaChat by Sber's
  authorization key, "Alice" (Alice AI LLM and YandexGPT) by key and catalog of Yandex AI Studio.
- **"Add your own".** Ollama, LM Studio, any OpenAI-compatible address and your own ACP agent by
  launch command. Each provider's key is stored separately and goes only to that provider.
- **"Found on this computer".** Ollama, LM Studio and your own server addresses are found automatically,
  with a "Scan again" button.
- **A subscription limit circle** at the bottom of the left panel: how much is left for 5 hours and for the week and when
  it resets. Checking the limits doesn't use up the subscription itself.
## [0.2.6] — 2026-11-09

### Added

- **"Connect your own AI".** A new window with a list of providers: ChatGPT, Claude, Gemini, Grok,
  GitHub Copilot, Cursor, Cline, Qwen Code, Pi, Devin, OpenCode, Goose and search across all agents
  in the ACP registry, plus DeepSeek and OpenRouter by key. A single "Download" button installs the agent, finds
  a sign-in already on the computer and pulls in its models. No sign-in — "Connect", and the agent
  opens the browser itself. In the card: sign-in, version and update, models with switches, "default
  for new bots".
- **Bots through external agents (ACP).** A bot can work through Claude Code, Codex and other
  agents: the progress shows as lines in the chat, the agent's permissions arrive as regular cards and
  respect the bot's mode, the agent reads and writes files only in the bot's folder, and our keys are invisible to it.
## [0.2.5] — 2026-11-09

### Added

- **Language choice on first launch.** Before sign-in the app asks for a language: Russian or English
  (with arrows or the mouse). The labels change to the chosen language at once, sign-in opens already in it,
  and the starter bots are translated. The screen doesn't appear again; the language is changed in settings.
- **"Build your own AI team".** After the first sign-in the app offers two paths: "Deloist subscription"
  (100 ₽ as a gift, nothing to set up) or "Connect your own AI" (the model settings open).
  There is one gift per computer: a new email on the same machine doesn't get a second one — the subscription is connected
  without the gift, and the app immediately offers to top up the balance.
- **Deloist subscription.** A new way to get replies: bots reach models through the Deloist site with an account token,
  without their own keys; the cost is deducted from the ruble balance. The balance is visible in the account window, and the model
  (fast or strong) is chosen in settings. When the money runs out, the bot says so directly.
- **Balance top-up.** In the account window, buttons for 100, 300 and 1000 ₽: payment opens in the browser on
  deloist.ru through YooKassa, like paying for the site's plans; back in the window, the person sees the new balance.

### Changed

- The sign-in code on the sign-in screen is set in the brand font, not Consolas.

## [0.2.4] — 2026-11-09

### Added

- **Sign-in to a Deloist account.** Without signing in, the app shows only the sign-in screen. You can sign in with a code
  from an email, without a password: a new person gets an account created automatically. You can also use VK ID or Yandex:
  the app opens deloist.ru, the person signs in there as usual and presses "Allow". The field already
  understands a phone number and replies that sign-in by number is coming soon. Account and sign-out — a button at the bottom of the
  sidebar. The device token lives in `secrets.json`, never reaches the window and is revoked on the site at sign-out.
- **A desktop shortcut.** On first launch the app puts a "Deloist Bots" shortcut on the
  desktop by itself, once: a deleted shortcut doesn't come back. It can be restored with a button in the account window. Someone else's shortcut
  with the same name (for example, of a site from the browser) is not overwritten by the app.

## [0.2.3] — 2026-11-08

### Changed

- **Live avatars.** Anime avatars lean toward the cursor, bounce on hover, nod at a click and
  shake their head if poked often. While you write to a bot, it listens to the input box, nods on send,
  and bounces when it finishes work. After two minutes of silence it dozes off and wakes up with the mouse.
  Slime listens to the input box and nods too.
- The desktop pet follows the cursor across the whole screen, not only over its own window.

## [0.2.2] — 2026-11-08

### Added

- **Command isolation.** `run_command` goes through a sandbox: a restricted token, a job object
  and a text path filter by default; AppContainer with its own file isolation — with
  `CREW_SANDBOX=strict`, and without the cage commands don't run in that mode. A self-check
  with a canary and the reason in `sandbox::status()`.
- **MCP over HTTP.** Streamable HTTP and SSE alongside stdio: `Mcp-Session-Id`, reinitialization
  of a stale session, transport choice in the settings form, a badge in the list.
- **Native Anthropic and Gemini drivers.** The Messages API and `streamGenerateContent`, their own
  conversions of history and tools, cache counters; token accounting and limits on them work too.
  Each provider has its own key: one provider's key never goes to another.
- **Browser.** `browser::backend()` (`chrome`/`edge`/`webview2`), search for Chrome, Edge, Chromium and Brave,
  `browser_path` validation, clear errors about the debugging port and Low integrity, proper closing
  of a dead session. `CREW_BROWSER=auto|external|embedded`; the built-in WebView2 host is prepared
  but not enabled — the CDP port could not be confirmed in this environment.

### Postponed

- History compression and memory with search: without a user setting they break existing
  tests on the number of model requests and prefix stability.
- The built-in WebView2 browser: needs a host window in `desktop.rs` and a check in a regular environment.

## [0.2.1] — 2026-11-07

### Changed

- Desktop pets one at a time: each bot can be brought out separately, the position is remembered,
  with no frame and no background.
- Small avatars are crisper: the frame is downscaled once in good quality, and small ones show the head.

## [0.2.0] — 2026-11-07

### Added

- **Command line** `deloist`: `bots`, `rooms`, `run`, `wait`, `team export`, `team apply`,
  exit codes 0–4, `--json` and `--export` of only changed files.
- **MCP server** `deloist --mcp`: `list_bots`, `list_rooms`, `read_thread`, `send_message`,
  `send_to_room`, `list_routines` — without confirmations, deletion or settings.
- **Images**: the `generate_image` tool, a daily counter, protection against a key leaking in error text.
- **Provider cache**: a stable system prompt prefix, the clock as a separate message,
  a history window anchor, sorted tool schemas, showing the cache share.
- **Groups**: a lead, an `@Name` mention, an `auto` mode; a shared thread and their own rules.
- **Skills**: blocks of rules that a bot receives when the task matches.
- **MCP client**: stdio servers as `mcp__<server>__<tool>` tools.
- **Webhooks**: starting a routine with an external POST with a secret in the address.
- **Message queue**: editing, reordering, "send into the current run", a pause after stop,
  recovery after a restart.
- **Fallback model**: a switch on a provider's limit, balance or overload.
- **Quiet routines**: the `[QUIET]` marker, removing the run from history, a log with the `quiet` status.
- **Night shift**: a schedule, a window, missed runs, a daily token limit.
- **Live avatars**: WebGL2, "slime" and "glass" materials, emotions, a face atlas.
- **Browser**: a real Chrome/Edge with the bot's profile, a page snapshot, mouse, keyboard,
  devtools, handing over to a person, permission for a site.
- **Updates**: a signed ed25519 manifest, updating connected programs.
- **A team in one file**: export and import without keys, memory or correspondence.
- Autostart with Windows and sleep prevention while routines are on.
- Demo driver: the full loop without keys.

### Changed

- The product was renamed to Deloist.
- On close the window goes to the tray and unloads WebView2.

### Fixed

- Attacks on the local server: CSRF, DNS rebinding, request type spoofing, reading without a session.
- The provider cache no longer breaks on groups and after quiet routines.

## [0.1.0] — 2026-11-06

### Added

- The first Rust version: a WebView2 window, tray, HTTP + SSE, threads, workspaces, memory,
  confirmations, routines.
