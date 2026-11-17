# Если что-то не работает

Начинайте с `sudo styk doctor`: он проверяет Docker, Compose, агент и его сокет, здоровье панели, базы и Caddy, ответ `STYK_PUBLIC_URL/healthz`, срок сертификата, место на диске, ротацию логов Docker и наличие новой версии. Каждая красная строка подсказывает, куда смотреть.

| Команда | Что |
|---|---|
| `styk status` | версия, образ, агент, итог последнего обновления, контейнеры |
| `styk logs panel` / `db` / `caddy` | логи контейнеров |
| `styk logs agent` | `journalctl -u styk-agent` |
| `styk logs update` | лог последнего обновления |

## Установка

| Симптом | Причина и что делать |
|---|---|
| `release signature is not valid` | файл скачался не полностью или подменён: повторите; если повторяется, не ставьте и проверьте, откуда берётся релиз (`STYK_REPO`) |
| `Debian or Ubuntu only`, `systemd is required` | другая ОС или контейнер без systemd |
| `ports 80/443 are taken` | там уже прокси: см. [own-proxy.md](own-proxy.md) |
| `the panel did not become healthy` | смотрите напечатанные логи; частое: не скачался образ (нет доступа к ghcr.io), мало памяти |
| `.env exists` | панель уже стоит: `styk update`, либо `styk uninstall --purge` и заново |

## Сертификат не выпускается

- `styk logs caddy`: ошибки ACME.
- A-запись указывает на этот сервер: `dig +short panel.example.com`.
- Порт 80 открыт снаружи (HTTP-01 challenge), в том числе в облачном firewall провайдера.
- За Cloudflare с проксированием: включите Full (strict) или на время выпуска DNS-only.
- Лимиты Let's Encrypt: много неудачных попыток подряд блокируют домен на час.

## Панель не открывается

1. `styk doctor`: контейнер `healthy`? Если нет, `styk logs panel`.
2. Через свой прокси: прокси в сети `STYK_PROXY_NETWORK`? `docker network inspect <сеть>` должен показать и прокси, и `styk-panel-panel-1`.
3. `curl -s http://127.0.0.1:8080/healthz` с хоста не сработает: порт не публикуется. Проверяйте из сети: `docker run --rm --network <сеть> curlimages/curl -s http://styk-panel:8080/healthz`.

## Агент

| Симптом | Что делать |
|---|---|
| в панели «агент недоступен» | `systemctl status styk-agent`, `styk logs agent` |
| `no /run/styk-agent/agent.sock` | агент не стартовал; после правки `/etc/styk-agent.env` нужен `systemctl restart styk-agent` |
| permission denied на сокет | `STYK_AGENT_GID` в `/opt/styk-panel/.env` не равен `getent group styk-agent`; исправьте и `styk restart` |
| файловый менеджер не видит каталог | добавьте корень в `STYK_AGENT_FS_ROOTS` |

## Обновление

| Итог в `styk status` | Что значит |
|---|---|
| `download or verification failed` | ничего не менялось; проверьте сеть и `styk logs update` |
| `styk-agent did not start` | возвращён старый агент; `journalctl -u styk-agent` по новому |
| `panel health check failed, rolled back` | работает прежняя версия; в логе обновления последние 60 строк панели |
| `another update is running` | дождитесь первого или проверьте `systemctl status styk-update` |

Кнопка «Обновить» в панели не появляется: `STYK_UPDATE_REPO` пуст или сервер не достаёт до api.github.com.

## Вход

- Забыли пароль: `sudo styk passwd <логин>`.
- Потеряли TOTP: `cd /opt/styk-panel && docker compose exec panel styk-panel user reset-totp <логин>`, при следующем входе по паролю TOTP настраивается заново.
- «Слишком много попыток»: лимит 5 на логин и 20 на IP за 15 минут, подождите.
- Заблокировали свой IP в «Доступ по IP»: с консоли провайдера `nft delete table inet styk_access` и проверьте список в панели.

## Место на диске

- `docker system df`, страница Docker → Очистка.
- Логи контейнеров без лимита: настройте ротацию (см. [install.md](install.md)).
- Старые дампы в `/var/backups/styk-panel`: ежедневные чистятся сами через 14 дней, `manual` и `preupdate` нет.
