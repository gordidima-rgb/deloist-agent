# Deloist in the cloud on a free Instabox machine — runbook

For a person or a model that connects Deloist to the user's own free Instabox machine.
Follow the steps exactly, in order. Do not improvise API calls: everything is inside the script.

## Rules (read first)
1. Run only `node tools/instabox/instabox.mjs <step>` from the repository root. Do not call `api.instabox.sh` yourself.
2. Never print, paste or log the contents of `~/.deloist/instabox/` (`api-key`, `claim.json`, `gateway.json`). They are secrets.
3. Never paste the `/auth/gateway?t=…` link anywhere. It opens the cloud for anyone for 30 seconds.
4. The person presses **Approve** in Instabox. You do not sign in for them and do not type their passwords.
5. `delete --yes` only after the person clearly says yes. It destroys the cloud copy and its data.
6. Every step is safe to repeat. On `ERROR`, do what the `FIX:` line says, then run the same step again. Do not edit the script.
7. Do not run `suspend`/`stop` through the API. A machine paused by hand does **not** wake on messages.

## Steps
Requires Node 20+ (`node --version`). Nothing to install.

| # | Command | What happens | Success line |
|---|---|---|---|
| 1 | `node tools/instabox/instabox.mjs signin` | Opens the approval page in the system browser and waits up to 15 min | `OK: approved by …` or `OK: Instabox key is already saved and works` |
| 2 | `node tools/instabox/instabox.mjs deploy` | Creates the machine `deloist` (template `agent-browser`), opens port 8080 to the internet, starts the Deloist server image | `OK: Deloist is running in the cloud` |
| 3 | `node tools/instabox/instabox.mjs open` | Wakes the machine if needed and opens the cloud Deloist in the browser | `OK: opened the cloud Deloist…` |

Or all three at once: `node tools/instabox/instabox.mjs all`.

Other commands: `status` (state, forced pause, address), `update` (pull the image again and restart; data stays), `delete --yes`.

### What to tell the person during step 1
> A page of Instabox opened in your browser. Sign in with Google or GitHub if it asks. Check that the code is **XXXX-XXXX** (from the script output) and the name is **Deloist**, then press **Approve**. If after signing in you see the list of machines instead, tell me — I will open the link again.

## Errors
| Code | Meaning | Do |
|---|---|---|
| `NO_KEY`, `KEY_REJECTED` | Not signed in, or the key was revoked | run `signin` |
| `CLAIM_EXPIRED`, `DENIED`, `SIGNIN_WAIT` | The approval did not happen | run `signin` again, repeat the message to the person |
| `LIMIT` | The free plan already has another machine | ask the person whether to delete the other machine in dashboard.instabox.sh; do not delete it yourself |
| `COOLDOWN` | Forced pause of the free plan (after 2–8 h of work) | wait the given seconds, run the same step again |
| `NETWORK` | No internet | check the connection, repeat |
| `DEPLOY` | The server did not start | run `deploy` again; if it repeats, send the whole output to the developers |
| anything else | — | run the same step again once; then send the output to the developers |

## How it works (for developers)
- **Machine.** Free plan: 1 machine, 4 vCPU / 8 GiB, 20 GiB persistent `/data`, Debian 13 x86_64 with Docker 29. Exec runs as `user` (sudo, docker).
- **Server.** Image `ghcr.io/gordidima-rgb/deloist-server:<tag>` (default `:stand`, the only tag in GHCR on 16.11.2026). It runs as uid 10001. Its data folder is `/data/deloist/data`, owned by 10001. The env file `/data/deloist/cloud.env` has mode 0600. The container has `--restart=always`.
- **Cloud mode env.**
  - `CREW_CLOUD=1`, `HOST=0.0.0.0`, `PORT=8080`;
  - `CREW_GATEWAY_PUBKEY` (raw Ed25519, hex) and `CREW_USER_ID=owner`;
  - `CREW_PUBLIC_ORIGIN` — the port URL `https://8080-<id>.instabox.tech`;
  - `CREW_SECRETS_KEY` (64 hex).
- **Login.** This computer is the gateway: it keeps the Ed25519 private key and signs a 30-second one-time pass `{uid, aud, iat, exp, nonce}` over `"deloist-gw-v1\n" + payload`. It is the same contract as `notes/web-version.md`.
- **Proxy.** Instabox's proxy sends `Host: 127.0.0.1:<port>` and the real host in `X-Forwarded-Host`.
- **Sleep.** The machine sleeps after 30 min without exec, SSH, CPU load or public-port requests (fixed on the free plan). A request to the public port wakes it (waits up to 45 s, else 503 + Retry-After). Measured 16.11.2026: idle sleep came exactly at 30 min (Deloist does not keep the machine awake); waking by a public request took ~72 s (first request 503 after 46 s, the retry answered after 26 s), so the first Telegram message after sleep is answered by Telegram's retry in ~1–2 min. After 2–8 h of continuous work there is a forced pause of 5 min to 2 h (409 `machine_cooldown`). Processes and sessions survive sleep.
- **Telegram.** In cloud mode the bot uses a webhook to `/hooks/telegram/<bot>` with a secret header, so Telegram's own retries wake the machine (branch `feat/telegram-webhook`).
