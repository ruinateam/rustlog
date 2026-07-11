# ChatTiers Rustlog

Rustlog - backend для логов Twitch-чата. Он пишет сообщения в ClickHouse, отдает API и веб-страницы с логами, а также умеет дозагружать историю из зеркал rustlog/justlog.

Этот репозиторий сейчас используется как локальный backend для ChatTiers в WSL.

## Текущая схема запуска

Основной путь проекта в Windows:

```text
C:\coding\projects\ChatTiers\rustlog
```

Тот же путь внутри WSL:

```bash
/mnt/c/coding/projects/ChatTiers/rustlog
```

В WSL включен `systemd`. После старта WSL автоматически поднимаются:

```bash
rustlog.service
clickhouse-server.service
```

Проверка:

```bash
systemctl is-active rustlog.service clickhouse-server.service
```

`rustlog.service` запускает release-бинарь:

```bash
/mnt/c/coding/projects/ChatTiers/rustlog/target/release/rustlog
```

## Автозапуск WSL

WSL поднимается в фоне через Windows registry key:

```text
HKCU\Software\Microsoft\Windows\CurrentVersion\Run
```

Текущая команда автозапуска:

```powershell
powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "Start-Process wsl.exe -ArgumentList '-d Ubuntu-24.04 --cd ~ sleep infinity' -WindowStyle Hidden"
```

Это держит WSL-дистрибутив живым после входа в Windows. Дальше `systemd` внутри WSL сам стартует нужные сервисы, включая `rustlog.service` и `clickhouse-server.service`.

Проверка из PowerShell:

```powershell
reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run
wsl.exe -d Ubuntu-24.04 -- systemctl is-active rustlog.service clickhouse-server.service
```

Важные настройки WSL:

```ini
[boot]
systemd=true

[interop]
appendWindowsPath = false
```

`appendWindowsPath = false` нужен, чтобы Windows PATH не ломал запуск Linux-команд и сервисов.

Для стабильного старта `rustlog.service` в WSL используется systemd drop-in с proxy env:

```text
/etc/systemd/system/rustlog.service.d/proxy.conf
```

Содержимое:

```ini
[Service]
Environment=HTTP_PROXY=http://172.30.96.1:10808
Environment=HTTPS_PROXY=http://172.30.96.1:10808
Environment=ALL_PROXY=http://172.30.96.1:10808
Environment=NO_PROXY=localhost,127.0.0.1,::1
```

Без этого сервис может падать на получении Twitch OAuth token до того, как откроет web-порт `8026`.

## Доступы

Backend:

```text
http://localhost:8026
```

ClickHouse:

```text
http://localhost:8123
```

## Сборка и деплой backend

Перед сборкой backend нужно собрать встроенный web UI из папки `web/`. Результат попадает в `web/dist` и вшивается в Rust-бинарь на этапе `cargo build`.

```bash
cd /mnt/c/coding/projects/ChatTiers/rustlog/web
yarn install
yarn build
```

На диске `C:` мало места, поэтому Rust release лучше собирать в Linux-раздел WSL, а потом копировать готовый бинарь в рабочий путь сервиса.

```bash
cd /mnt/c/coding/projects/ChatTiers/rustlog

CARGO_TARGET_DIR=/home/linar/rustlog-target \
CARGO_INCREMENTAL=0 \
cargo build --release -j1

cp /home/linar/rustlog-target/release/rustlog \
  /mnt/c/coding/projects/ChatTiers/rustlog/target/release/rustlog.new

mv /mnt/c/coding/projects/ChatTiers/rustlog/target/release/rustlog.new \
  /mnt/c/coding/projects/ChatTiers/rustlog/target/release/rustlog

sudo systemctl restart rustlog.service
```

Проверка после деплоя:

```bash
/mnt/c/coding/projects/ChatTiers/rustlog/target/release/rustlog --help
systemctl status rustlog.service --no-pager -l
```

Если перезапуск из обычного WSL-пользователя упирается в `Interactive authentication required`, перезапусти сервис из PowerShell под root:

```powershell
wsl.exe -d Ubuntu-24.04 -u root -- systemctl restart rustlog.service
```

## Web UI

Встроенный frontend находится в папке:

```bash
/mnt/c/coding/projects/ChatTiers/rustlog/web
```

Это Vite + React приложение. Для разработки:

```bash
cd /mnt/c/coding/projects/ChatTiers/rustlog/web
yarn install
yarn start
```

Dev server берет backend API из `web/.env.development`:

```text
VITE_API_BASE_URL=http://localhost:8026
```

Production-сборка:

```bash
cd /mnt/c/coding/projects/ChatTiers/rustlog/web
yarn build
```

После `yarn build` нужно заново собрать Rust-бинарь, потому что `src/web/frontend.rs` встраивает `web/dist` в executable:

```rust
#[folder = "$CARGO_MANIFEST_DIR/web/dist"]
```

Отдельный systemd service для `web/` не нужен в production: статику отдает сам `rustlog.service`.

## Логи сервисов

Backend:

```bash
journalctl -u rustlog.service -f
```

ClickHouse:

```bash
journalctl -u clickhouse-server.service -f
```

## Загрузка логов из зеркал

Обычное зеркалирование одного канала/периода:

```bash
./target/release/rustlog mirror \
  --base-url https://logs.zonian.dev \
  --channel zakvielchannel \
  --year 2026 \
  --month 1 \
  --batch 1000
```

Загрузка из локального JSON-кэша:

```bash
./target/release/rustlog mirror \
  --local-cache /mnt/c/Users/Linar/Desktop/twitchlogs/cache \
  --channel zakvielchannel \
  --year 2026
```

Поддерживаемые фильтры:

```text
--channel      логин канала, обязательно
--base-url     корень API зеркала
--local-cache  путь к локальному cache/<channel>/daily/YYYY/MM/DD.json
--year         год
--month        месяц
--day          день
--batch        размер вставки в ClickHouse
--proxy        HTTP(S)-прокси для запросов к зеркалам
```

## Дозагрузка недостающих дней

Команда `fill-missing` берет список зеркал из:

```text
https://logs.zonian.dev/api/<channel>
```

Она сравнивает дни, доступные на зеркалах, с днями в локальном ClickHouse и загружает только отсутствующие.

Сначала dry-run:

```bash
./target/release/rustlog fill-missing \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney \
  --proxy http://172.30.96.1:10808 \
  --dry-run
```

Реальная загрузка:

```bash
./target/release/rustlog fill-missing \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney \
  --proxy http://172.30.96.1:10808
```

Можно исключить конкретное зеркало:

```bash
./target/release/rustlog fill-missing \
  --year 2026 \
  --channel linaryx \
  --exclude-instance https://logs.twitchmetrics.xyz
```

## Очистка дублей

Дедупликация идет по Twitch message id. Без `--execute` команда только показывает отчет.

Проверить дубли:

```bash
./target/release/rustlog cleanup-duplicate-ids \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney
```

Удалить дубли, оставив одну строку на `(channel_login, id)`:

```bash
./target/release/rustlog cleanup-duplicate-ids \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney \
  --execute
```

Перед `--execute` лучше всегда смотреть dry-run, потому что команда переписывает строки в ClickHouse через delete mutation и повторную вставку сохраненных строк.

## Прокси из WSL

Из WSL `127.0.0.1:10808` не является Windows-host proxy. Рабочий адрес:

```text
http://172.30.96.1:10808
```

Проверка HTTPS через прокси:

```bash
curl -I -x http://172.30.96.1:10808 https://logs.zonian.dev/api/linaryx
```

Если WSL-сеть поменяется, адрес Windows-host можно найти так:

```bash
ip route | awk '/default/ {print $3}'
```

## Конфиг

Runtime-конфиг лежит в `config.json`. Пример структуры и описание полей находятся в [docs/CONFIG.md](./docs/CONFIG.md).

Не коммить реальные Twitch/Supabase keys.

## Миграция и импорт старых логов

Для переноса старых justlog-файлов и исторической загрузки из зеркал см. [docs/MIGRATION.md](./docs/MIGRATION.md).

## Быстрая диагностика

Проверить, что сервисы живы:

```bash
systemctl is-active rustlog.service clickhouse-server.service
```

Проверить, какой бинарь запущен:

```bash
systemctl status rustlog.service --no-pager -l
```

Проверить свободное место:

```bash
df -h /mnt/c /home/linar
```

Проверить доступность API:

```bash
curl -I http://localhost:8026
```
