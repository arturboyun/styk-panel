# Styk Panel

Панель управления своим сервером: мониторинг, Docker и Compose (действия, логи, терминал, deploy-webhook, очистка), домены и Caddy, базы данных и бэкапы, файлы, алерты с uptime-проверками и Telegram, безопасность (SSH, firewall, обновления, доступ по IP), аудит. Интерфейс на русском.

Debian и Ubuntu, amd64 и arm64. Нужны только Docker и systemd; установщик ставит Docker сам, если его нет.

## Установка

```bash
curl -fsSL https://github.com/arturboyun/styk-panel/releases/latest/download/install.sh \
  | sudo bash -s -- panel --domain panel.example.com
```

Перед запуском направьте A-запись домена на сервер. Установщик:

1. проверяет подпись релиза (minisign) и контрольные суммы каждого файла;
2. ставит Docker, если его нет;
3. ставит `styk-agent` (systemd) и CLI `styk`;
4. создаёт `/opt/styk-panel` с `compose.yml` и `.env` (секреты генерируются на месте);
5. поднимает панель, Postgres и Caddy с сертификатом Let's Encrypt;
6. печатает логин и пароль администратора; TOTP настраивается при первом входе.

Если порты 80 и 443 уже заняты вашим прокси, Caddy не ставится: установщик подскажет, как направить домен на `styk-panel:8080` через Docker-сеть `styk-proxy`.

Опции после `panel`:

| Опция | Что делает |
|---|---|
| `--domain NAME` | адрес панели; встроенный Caddy получит для него сертификат |
| `--email ADDR` | контакт для Let's Encrypt |
| `--no-caddy` | не ставить встроенный Caddy (есть свой прокси) |
| `--proxy-network NAME` | Docker-сеть, общая с вашим прокси (по умолчанию `styk-proxy`) |
| `--version vX.Y.Z` | конкретный релиз вместо последнего |

## Обновление и обслуживание

Панель раз в 6 часов проверяет GitHub Releases. Когда выходит новая версия, в сайдбаре появляется метка, а в **Настройки → Версия и обновления** кнопка «Обновить». То же на сервере:

```bash
styk update            # последний релиз; styk update v0.2.0 для конкретного
```

Обновление:

1. проверяет подпись;
2. сохраняет дамп базы панели в `/var/backups/styk-panel`;
3. меняет агент и образ (образ закреплён по digest из подписанного файла);
4. ждёт healthcheck, а если новая версия не поднялась, сам возвращает прежнюю.

| Команда | Что делает |
|---|---|
| `styk status` | версия, контейнеры, агент, итог последнего обновления |
| `styk doctor` | проверяет Docker, агент, панель, HTTPS, сертификат, диск, ротацию логов, версию |
| `styk logs [panel\|db\|caddy\|agent\|update]` | логи |
| `styk rollback` | вернуть версию, которая была до последнего обновления |
| `styk backup` | дамп базы панели сейчас |
| `styk passwd <user>` | новый случайный пароль пользователю |
| `styk restart` | перечитать `.env`, перезапустить панель и агент |
| `styk uninstall [--purge]` | удалить панель (`--purge` удаляет и данные) |

Панель сама делает ежедневный дамп своей базы (14 дней хранения), это отключается через `STYK_DB_CONTAINER=` в `.env`. Пользователи: `cd /opt/styk-panel && docker compose exec panel styk-panel user --help`.

## Как устроено

```
Browser ──▶ Caddy ──▶ styk-panel (Docker: axum + статический Next.js)
                          ├─ Postgres (сеть internal)
                          ├─ /var/run/docker.sock   список, логи, inspect, exec
                          └─ /run/styk-agent/agent.sock
                                   ▼
                          styk-agent (systemd на хосте): /proc, cgroup v2, файлы в песочнице,
                          journal, ss, nft; долгие задачи (compose, pg_dump, apt, обновление)
                          как transient-юниты systemd по белому списку
```

| Каталог | Что внутри |
|---|---|
| `crates/styk-proto` | протокол panel ↔ agent (JSON по строке через unix socket) |
| `crates/styk-agent` | хостовый агент: только типизированные методы, никаких произвольных команд |
| `crates/styk-panel` | веб-сервер, API, сбор метрик, CLI пользователей, миграции |
| `frontend` | Next.js 16 (static export), Tailwind 4, shadcn/ui (Base UI), uPlot |
| `deploy` | `install.sh`, CLI `styk`, Dockerfile, compose, systemd-юниты |

## Настройки

Панель: `/opt/styk-panel/.env`, после правки `styk restart`. Полный список в [env.example](deploy/env.example).

| Переменная | Зачем |
|---|---|
| `STYK_PUBLIC_URL` | адрес панели для ссылок в уведомлениях |
| `TELEGRAM_BOT_TOKEN`, `TELEGRAM_BOT_USERNAME` | Telegram Login и алерты: бот из @BotFather, `/setdomain` на домен панели |
| `CLOUDFLARE_API_TOKEN`, `STYK_PUBLIC_IP` | мастер «Новый сайт» создаёт A-запись (токен с правом Zone.DNS:Edit) |
| `STYK_CADDYFILE`, `STYK_CADDY_CONTAINER` | ваш Caddy вместо встроенного: путь к Caddyfile на хосте и имя контейнера |
| `STYK_PROTECTED_PROJECTS` | проекты, которые нельзя остановить из панели (по умолчанию `styk-panel`) |
| `STYK_HEARTBEAT_URL` | heartbeat раз в минуту во внешний мониторинг |
| `STYK_UPDATE_REPO` | откуда брать обновления; пусто, чтобы не проверять |

Агент: `/etc/styk-agent.env`, после правки `systemctl restart styk-agent`.

| Переменная | По умолчанию |
|---|---|
| `STYK_AGENT_FS_ROOTS` | `/opt,/srv,/var/backups:ro`: корни файлового менеджера (`:ro` только чтение) |
| `STYK_AGENT_DATA_DIR` | `/opt/styk-panel/data`: история версий файлов и корзина (30 дней) |
| `STYK_AGENT_BACKUP_DIRS` | `/var/backups/containers,/var/backups/styk-panel`: что показывает страница «Бэкапы» |
| `STYK_AGENT_UNITS` | `backup-all.service,backup-restore-test.service`: юниты, которые можно запускать из панели |
| `STYK_AGENT_PROTECTED` | `styk-panel`: проекты, которые нельзя остановить; Caddy только reload |

### Сайты за встроенным Caddy

Страница «Домены» добавляет сайты в `/opt/styk-panel/caddy/Caddyfile`. Приложение подключается к сети `styk-proxy` только тем контейнером, к которому ходит Caddy, и под уникальным именем:

```yaml
services:
  web:
    networks:
      default: {}
      styk-proxy: {aliases: [myproject-web]}
networks:
  styk-proxy: {external: true}
```

В Caddyfile используйте только такие имена (`reverse_proxy myproject-web:3000`). Короткие `web` или `api` Compose даёт каждому сервису, и в общей сети они совпадают у разных проектов. Панель предупреждает об этом и о базах в общей сети.

### Telegram

1. Создайте бота в @BotFather, `/setdomain` на домен панели, токен и имя впишите в `.env`, затем `styk restart`.
2. Привяжите аккаунт: `docker compose exec panel styk-panel user link-telegram <login> <telegram_id>`.
3. Напишите боту `/start`, в **Алерты → Уведомления** выберите чат и отправьте тест.

### Deploy-webhook

**Проекты → проект → Deploy webhook → Создать**, секрет показывается один раз. Из CI:

```bash
BODY='{}'   # или '{"services":["app"]}'
TS=$(date +%s)
SIG=$(printf '%s.%s' "$TS" "$BODY" | openssl dgst -sha256 -hmac "$STYK_HOOK_SECRET" -hex | sed 's/^.* //')
curl -fsS -X POST https://panel.example.com/api/hooks/deploy/<project> \
  -H "X-Styk-Timestamp: $TS" -H "X-Styk-Signature: sha256=$SIG" \
  -H 'content-type: application/json' -d "$BODY"
```

Запускается `docker compose pull && up -d`, лог в разделе «Операции» проекта.

## Безопасность

- **Вход:** пароль (argon2id) и обязательный TOTP либо Telegram Login для привязанных аккаунтов.
- **Сессии:** хранятся в Postgres.
  - cookie `__Host-` с HttpOnly, Secure и SameSite=Strict;
  - CSRF double-submit;
  - лимит попыток: 5 на логин и 20 на IP за 15 минут.
- **sudo:** все изменяющие действия и чтение секретов требуют повторного TOTP. Окно действует 30 минут без действий.
- **Аудит:** все входы и действия пишутся в аудит. TOTP-секреты зашифрованы AES-256-GCM (`STYK_SECRET_KEY`).
- **Контейнер панели:** без root, `read_only`, `cap_drop: ALL`. При этом `docker.sock` равен root на хосте, поэтому держите панель за HTTPS и TOTP и не открывайте лишнего.
- **Агент:** слушает только unix socket и принимает только типизированные методы. Файловый менеджер работает внутри своих корней. Перед записью файла сохраняется прежняя версия, удаление идёт в корзину. Задачи запускаются по белому списку.
- **Caddyfile:** правка идёт через `caddy validate`, запись и `caddy reload`; при ошибке файл возвращается.
- **Доступ по IP** (Безопасность → Доступ): SSH-allowlist и блок-лист в nft-таблице `inet styk_access`. Пустой allowlist включить нельзя, свой текущий IP заблокировать нельзя. Чтобы правила поднимались при загрузке, поставьте [`styk-access-firewall.service`](deploy/styk-access-firewall.service). Переключатель «Разрешить GitHub Actions» работает с таймером [`styk-access-github-refresh`](deploy/styk-access-github-refresh):

  ```bash
  install -m 0755 deploy/styk-access-github-refresh /usr/local/sbin/
  install -m 0644 deploy/styk-access-*.service deploy/styk-access-github-refresh.timer /etc/systemd/system/
  systemctl daemon-reload && systemctl enable --now styk-access-firewall styk-access-github-refresh.timer
  ```
- **Restore базы:** перед восстановлением делается страховочный дамп. SQL-консоль работает только на чтение, с таймаутом 10 с.
- **Релизы:** подписаны ключом minisign; публичная часть лежит в [deploy/minisign.pub](deploy/minisign.pub) и встроена в `install.sh` и `styk`. Неподписанный или изменённый файл не будет установлен.

## Разработка

```bash
docker run -d --name styk-panel-devdb -e POSTGRES_USER=styk -e POSTGRES_PASSWORD=styk -e POSTGRES_DB=styk -p 127.0.0.1:55499:5432 postgres:18-alpine
```

`.dev/env` (не в git):

```bash
export DATABASE_URL=postgres://styk:styk@127.0.0.1:55499/styk
export STYK_AGENT_SOCKET=$PWD/.dev/agent.sock
export STYK_SECRET_KEY=...        # openssl rand -base64 32, один раз
export STYK_COOKIE_SECURE=false
export STYK_LISTEN=127.0.0.1:8088
export STYK_STATIC_DIR=$PWD/frontend/out
```

```bash
. .dev/env && cargo build && ./target/debug/styk-agent &
./target/debug/styk-panel user create admin
./target/debug/styk-panel serve
cd frontend && NODE_ENV=production npm run build   # или npm run dev с прокси /api на :8088 (без WebSocket)
```

Проверки: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cd frontend && npx tsc --noEmit && npm run lint`.

Релиз: поднимите `version` в `Cargo.toml` и запушьте тег `vX.Y.Z`. [release.yml](.github/workflows/release.yml) соберёт бинарники для amd64 и arm64, мультиарх-образ в GHCR и подписанный релиз. Проверить установку без публикации: `scripts/local-release.sh dist v0.1.0`, затем `STYK_RELEASE_DIR=dist bash dist/install.sh panel ...` на тестовой машине.

## Лицензия

MIT
