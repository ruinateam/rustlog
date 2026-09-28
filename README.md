# ChatTiers Rustlog

Форк [rustlog](https://github.com/boring-nick/rustlog) — self-hosted логгер Twitch-чата. Пишет сообщения в ClickHouse, отдаёт REST API, встроенный веб-интерфейс и поток живых событий.

Отличия от апстрима:

- **Tiers** — рейтинг чаттеров по фиксированным окнам 1/5/15/30/60 минут с тирами HT1..LT5 и календарными периодами в `Europe/Moscow`.
- **API v2** — аддитивный контракт: canonical Twitch id, строгие диапазоны RFC 3339, ошибки `application/problem+json`.
- **Admin firehose** — WebSocket с живыми сообщениями, уже принятыми очередью writer.
- **Chat badges** — метаданные глобальных и канальных бейджей для отрисовки сохранённых сообщений.
- **Состояние в ClickHouse** — подключённые каналы и opt-out живут в таблицах, а не в JSON-конфиге.
- **Импорт истории** — `mirror`, `fill-missing` и `cleanup-duplicate-ids` с соблюдением opt-out.

Legacy-роуты и документация апстрима остаются доступными без изменений.

## Быстрый старт (Docker Compose)

Нужен Docker с плагином Compose.

```bash
git clone --recurse-submodules https://github.com/ruinateam/rustlog.git
cd rustlog

cp .env.example .env               # задай длинный случайный CLICKHOUSE_PASSWORD
cp config.dist.json config.json    # заполни clientID/clientSecret (Twitch application)

docker compose -f docker-compose.dev.yml up -d
docker compose -f docker-compose.dev.yml ps
```

После старта:

- backend: <http://localhost:8026>
- ClickHouse: <http://localhost:8123>
- документация: <http://localhost:8026/docs> и <http://localhost:8026/api/v2/docs>

Compose поднимает ClickHouse `26.5.6.64` с healthcheck и named volume, монтирует `config.json` в backend read-only, а сетевые поля конфига переопределяет через `RUSTLOG_*`. Twitch-credentials остаются в `config.json`.

## Локальная сборка

UI встраивается в бинарь, поэтому фронтенд собирается до `cargo build`:

```bash
cd web
yarn install
yarn build

cd ..
cargo build --release
./target/release/rustlog --config config.json
```

Требования: Rust 1.94 (как в Dockerfile), Node.js 18+ и yarn (`corepack enable`). Если репозиторий клонирован без `--recurse-submodules`, выполни `git submodule update --init --recursive`.

Проверки перед пушем:

```bash
cargo fmt -- --check
cargo test
cd web && yarn typecheck && yarn build
```

## Docker-образ

GitHub Actions публикует multi-architecture образ в GHCR при пуше вне pull request:

```bash
docker pull ghcr.io/ruinateam/rustlog:main
docker run --rm -p 8026:8026 \
  -v "$PWD/config.json:/app/config.json:ro" \
  ghcr.io/ruinateam/rustlog:main
```

Порт публикуй по `listenAddress` из конфига. Секреты в образ не попадают: `.dockerignore` исключает `config.json`, а сетевые поля можно переопределить через переменные окружения.

## Конфигурация

Runtime-конфиг — `config.json` (не коммитить). Пример — `config.dist.json`, описание полей — [docs/CONFIG.md](./docs/CONFIG.md).

```bash
./target/release/rustlog --config /path/to/config.json
```

Для контейнеров сетевые поля переопределяются переменными окружения: `RUSTLOG_CLICKHOUSE_URL`, `RUSTLOG_CLICKHOUSE_DB`, `RUSTLOG_CLICKHOUSE_USERNAME`, `RUSTLOG_CLICKHOUSE_PASSWORD`, `RUSTLOG_LISTEN_ADDRESS`.

Состояние каналов и opt-out после первой миграции хранится в ClickHouse. На один deployment должен быть только один backend, который его меняет: см. [docs/CONFIG.md](./docs/CONFIG.md#состояние-каналов-и-opt-out).

## API и live-поток

- Legacy: `/docs` и `/openapi.json`.
- v2: `/api/v2/docs` и `/api/v2/openapi.json`, краткий контракт — [docs/API_V2.md](./docs/API_V2.md).
- `GET /admin/firehose` — WebSocket для операционных клиентов. Требует заголовок `X-Api-Key` (ключ нельзя передавать query-параметром), отдаёт raw IRC без CRLF или `?format=json-basic`, отставший клиент закрывается кодом `1013`; пропуск дочитывается через HTTP API.
- `GET /metrics` — Prometheus-метрики.

## Tiers

Эндпоинты `/{channel_id_type}/{channel}/tiers/{year}[/{month}[/{day}]]`; тип канала — `channel` (логин) или `channelid` (Twitch id).

- Периоды календарные, расчёт в `Europe/Moscow`; окна — фиксированные интервалы 1/5/15/30/60 минут.
- `?mode=all|online|offline` — все сообщения, только во время стримов или только вне стримов. Окна стримов берутся из SullyGnome с фолбэком на кеш.
- `?exclude_bots=<login,...>` — какие логины исключать; по умолчанию вырезается набор распространённых ботов.
- При равных значениях порядок определяется `user_id`; в ответе не больше 500 записей.
- Для `offline` unique-сообщения считаются по фактическим сообщениям, а не как разность агрегатов.

## Импорт истории и обслуживание

Зеркалирование одного периода:

```bash
./target/release/rustlog mirror \
  --base-url https://logs.zonian.dev \
  --channel <channel> \
  --year 2026 --month 1
```

`fill-missing` ищет недостающие дни по зеркалам из `https://logs.zonian.dev/api/<channel>`:

```bash
./target/release/rustlog fill-missing --year 2026 --channel <channel> --dry-run
./target/release/rustlog fill-missing --year 2026 --channel <channel>
```

Полезные флаги: `--local-cache`, `--proxy` (повторяемый), `--rps`, `--http-concurrency`, `--exclude-instance`; для `fill-missing` — `--repair-existing` и `--deep`.

Дубли Twitch message id:

```bash
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel>
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel> --execute
```

Без `--execute` команда только показывает отчёт; с ним переписывает строки в ClickHouse (delete mutation и повторная вставка). Перед `--execute` сделай бэкап и проверь dry-run.

Перенос старых justlog-файлов и правила opt-out при импорте описаны в [docs/MIGRATION.md](./docs/MIGRATION.md).

## CI

`.github/workflows/docker-publish.yml` на push в `main` и pull request запускает quality-джобу (`cargo fmt -- --check`, `cargo test`, `yarn typecheck`, `yarn build`), а вне pull request собирает и публикует multi-arch образ в GHCR.

## Лицензия

MIT — см. [LICENSE](./LICENSE).
