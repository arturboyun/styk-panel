# Панель за своим прокси

Если на сервере уже есть Caddy, nginx или Traefik на 80/443, установщик не ставит встроенный Caddy и печатает, куда направить домен. Панель слушает `styk-panel:8080` в Docker-сети `STYK_PROXY_NETWORK`.

## Установка

```bash
curl -fsSL https://github.com/arturboyun/styk-panel/releases/latest/download/install.sh \
  | sudo bash -s -- panel --domain panel.example.com --no-caddy --proxy-network edge
```

`--proxy-network` укажите ту сеть, где уже живёт ваш прокси. Если её нет, она будет создана, и прокси надо подключить к ней:

```bash
docker network connect edge <контейнер прокси>
```

Для прокси в Compose лучше объявить сеть в его `compose.yml`, чтобы подключение пережило пересоздание:

```yaml
networks:
  edge: {external: true}
```

## Конфиг прокси

Caddy:

```caddy
panel.example.com {
	encode zstd gzip
	reverse_proxy styk-panel:8080
}
```

nginx (WebSocket нужен терминалу и живым логам):

```nginx
server {
    listen 443 ssl http2;
    server_name panel.example.com;
    # ssl_certificate ...;
    location / {
        proxy_pass http://styk-panel:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
        proxy_read_timeout 1h;
    }
}
```

с `map $http_upgrade $connection_upgrade { default upgrade; '' close; }` в блоке `http`.

Панель доверяет `X-Forwarded-For` (`STYK_TRUST_PROXY=true` в compose): лимиты входа и аудит видят адрес клиента. Поэтому не публикуйте порт 8080 наружу в обход прокси.

## Страница «Домены» со своим Caddy

Если ваш прокси это Caddy, панель может править его Caddyfile. В `/opt/styk-panel/.env`:

```bash
STYK_CADDYFILE=/opt/edge/caddy/Caddyfile     # путь на хосте
STYK_CADDY_CONTAINER=edge-caddy-1            # для caddy validate и reload
STYK_PROTECTED_PROJECTS=styk-panel,edge      # Compose-проекты, которые нельзя остановить из панели
```

и в `/etc/styk-agent.env` то же для агента:

```bash
STYK_AGENT_PROTECTED=styk-panel,edge
```

Затем `sudo styk restart`. Монтируйте в Caddy каталог, а не сам файл: панель сохраняет Caddyfile через rename, и примонтированный файл перестанет обновляться в контейнере.

Если прокси не Caddy, оставьте `STYK_CADDYFILE` пустым: «Домены» будут недоступны, остальное работает.

## Cloudflare

- SSL/TLS режим **Full (strict)**.
- Статика панели отдаётся с `Cache-Control: immutable`, но Cloudflare по умолчанию заменяет его на 4 часа. В зоне: **Caching → Configuration → Browser Cache TTL → Respect Existing Headers**.
- WebSocket в Cloudflare включён по умолчанию; простой больше 100 секунд он обрывает, панель переподключается сама.

## Перевести существующую ручную установку на `styk`

Если панель уже стоит в `/opt/styk-panel` (например, старым скриптом деплоя):

1. Сохраните копии: `cp /opt/styk-panel/.env{,.bak-$(date +%F)}`, `cp /etc/styk-agent.env{,.bak-$(date +%F)}`, `docker compose exec -T db pg_dump -U styk -d styk -Fc > /root/styk-before.dump`.
2. Впишите в `.env` всё, что отличается от умолчаний: `STYK_CADDYFILE`, `STYK_CADDY_CONTAINER`, `STYK_PROTECTED_PROJECTS`, `STYK_PUBLIC_URL`, `STYK_PROXY_NETWORK`, `COMPOSE_PROFILES=` (пусто), `STYK_DB_CONTAINER` (пусто, если базу дампит общий скрипт). В `/etc/styk-agent.env`: `STYK_AGENT_PROTECTED`, `STYK_AGENT_BACKUP_DIRS`, `STYK_AGENT_FS_ROOTS`, если меняли.
3. Поставьте CLI из релиза, проверив подпись:
   ```bash
   cd "$(mktemp -d)"
   base=https://github.com/arturboyun/styk-panel/releases/latest/download
   curl -fsSLO "$base/SHA256SUMS" -O "$base/SHA256SUMS.minisig" -O "$base/styk"
   minisign -Vm SHA256SUMS -P RWSO3zetXeG/q4VDqK5P45251fcvQU8xgg0mW2H3YZ2sbF1oKfvXDnQ/   # ключ из deploy/minisign.pub
   grep '  styk$' SHA256SUMS | sha256sum -c - && sudo install -m 0755 styk /usr/local/bin/styk
   ```
4. `sudo styk update`: `compose.yml` заменится на релизный, образ закрепится по digest, агент обновится.
5. `styk doctor`.
