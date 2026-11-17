# Установка

Подходит для Debian 12+ и Ubuntu 22.04+ на amd64 или arm64 с systemd. Docker ставить заранее не нужно.

## Перед запуском

1. Направьте A-запись домена (например `panel.example.com`) на IP сервера. Если домен за Cloudflare, на время выпуска сертификата можно оставить DNS-only.
2. Откройте входящие 80 и 443 (TCP) и 443 (UDP, для HTTP/3).
3. Проверьте, свободны ли порты: `ss -ltn '( sport = :80 or sport = :443 )'`. Если там уже есть ваш прокси, читайте [own-proxy.md](own-proxy.md).

## Запуск

```bash
curl -fsSL https://github.com/arturboyun/styk-panel/releases/latest/download/install.sh \
  | sudo bash -s -- panel --domain panel.example.com --email you@example.com
```

| Опция | Что делает |
|---|---|
| `--domain NAME` | адрес панели; встроенный Caddy получит для него сертификат |
| `--email ADDR` | контакт для Let's Encrypt |
| `--no-caddy` | не ставить встроенный Caddy |
| `--proxy-network NAME` | Docker-сеть, общая с прокси (по умолчанию `styk-proxy`) |
| `--version vX.Y.Z` | конкретный релиз вместо последнего |

Конкретную версию можно ставить и через её собственный `install.sh`:
`https://github.com/arturboyun/styk-panel/releases/download/vX.Y.Z/install.sh`.

## Что происходит

1. `install.sh` ставит `curl`, `minisign`, `openssl`, `iproute2`, скачивает `SHA256SUMS` и подпись, проверяет подпись встроенным ключом, проверяет `styk` по сумме и передаёт работу `styk setup`.
2. `styk setup` проверяет ОС, ставит Docker из download.docker.com (если не вышло, из пакетов дистрибутива), скачивает и проверяет все файлы релиза.
3. Ставит `styk-agent` в `/usr/local/bin`, юнит `styk-agent.service`, группу `styk-agent` и `/etc/styk-agent.env`.
4. Создаёт `/opt/styk-panel`: `compose.yml`, `.env` с секретами, сгенерированными на месте (`chmod 600`), и `caddy/Caddyfile`.
5. Поднимает Postgres, панель и Caddy, ждёт healthcheck до 3 минут.
6. Создаёт пользователя `admin` и печатает его пароль один раз. При первом входе панель попросит настроить TOTP.

Если на шаге 5 что-то не поднялось, установщик печатает последние логи. Повторный запуск при существующем `/opt/styk-panel/.env` откажется: для уже установленной панели используйте `styk update`.

## Где что лежит

| Путь | Что |
|---|---|
| `/opt/styk-panel/.env` | настройки панели, после правки `styk restart` |
| `/opt/styk-panel/compose.yml` | перезаписывается при обновлении, не правьте |
| `/opt/styk-panel/caddy/Caddyfile` | встроенный Caddy; сайты добавляет страница «Домены» |
| `/etc/styk-agent.env` | настройки агента, после правки `systemctl restart styk-agent` |
| `/var/lib/styk` | версия, итог и лог последнего обновления, данные для отката |
| `/var/backups/styk-panel` | дампы базы панели |
| Docker volume `styk-panel_db` | база панели |
| Docker volume `styk-panel_caddy_data` | сертификаты Caddy |

## После установки

1. Войдите под `admin`, настройте TOTP.
2. Ограничьте размер логов Docker, если `styk doctor` предупреждает об этом:
   ```json
   {"log-driver":"json-file","log-opts":{"max-size":"10m","max-file":"3"}}
   ```
   в `/etc/docker/daemon.json`, затем `systemctl restart docker`.
3. По желанию: Telegram (см. README), `STYK_HEARTBEAT_URL` для внешнего мониторинга, доступ по IP.
4. Запустите `styk doctor`: всё должно быть зелёным.

## Офлайн и тесты

`STYK_RELEASE_DIR=/путь/к/релизу` берёт файлы из каталога вместо GitHub. Подпись проверяется так же. Если рядом лежит `image.tar`, образ загружается из него без проверки подписи, поэтому так делайте только на тестовых машинах. Как собрать такой каталог, описано в [releasing.md](releasing.md).

## Удаление

```bash
styk uninstall            # контейнеры и агент; база, /opt/styk-panel и дампы остаются
styk uninstall --purge    # также база, сертификаты, /opt/styk-panel и /var/lib/styk
```

Дампы в `/var/backups/styk-panel` не удаляются ни в каком режиме.
