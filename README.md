# ChatTiers Rustlog

Форк [rustlog](https://github.com/boring-nick/rustlog) - селф-хост логгер Twitch-чата. Пишет сообщения в ClickHouse, отдаёт по REST API, имеет встроенный веб-интерфейс и реал-тайм событиями.

Отличия от [rustlog](https://github.com/boring-nick/rustlog):

- **Tiers** - рейтинг чаттеров по фиксированным окнам 1/5/15/30/60 минут с тирами HT1..LT5 и календарными периодами в `Europe/Moscow`.
- **API v2** - Twitch ID, строгие диапазоны RFC 3339, ошибки `application/problem+json`.
- **Admin Firehose** - WebSocket API для реал-тайм сообщений.
- **Значки чата** - метаданные глобальных и канальных значков для отрисовки сохранённых сообщений.
- **Состояние в ClickHouse** - подключённые каналы и opt-out живут в таблицах, а не в JSON-конфиге.
- **Импорт истории** - `mirror`, `fill-missing` и `cleanup-duplicate-ids` с соблюдением opt-out.

## Быстрый старт

Для старта понадобится [Docker](https://www.docker.com/get-started). Далее, запустите следующие команды:

```bash
# Склонируйте и перейдите в директорию
git clone --recurse-submodules https://github.com/ruinateam/rustlog.git
cd rustlog

# Скопируйте настройки и заполните поля 
cp .env.example .env
cp config.dist.json config.json

# Запустите контейнер
docker compose -f docker-compose.dev.yml up -d
docker compose -f docker-compose.dev.yml ps
```

После старта вам станут доступны:

- Бэкенд - [localhost:8026](http://localhost:8026)
- ClickHouse - [localhost:8123](http://localhost:8123)
- Документация - [localhost:8026/docs](http://localhost:8026/docs) и [localhost:8026/api/v2/docs](http://localhost:8026/api/v2/docs)

Docker поднимает ClickHouse `26.5.6.64` с healthcheck-ом, монтирует `config.json` в бэкенд в режиме read-only, а сетевые поля настроек переопределяет через `RUSTLOG_*`. Данные от Twitch-а остаются в `config.json`.

## Локальная сборка

> [!NOTE]
> Если репозиторий клонирован без `--recurse-submodules`, сначала выполните `git submodule update --init --recursive`.

Требования для сборки:

- [rustup](https://rustup.rs): нужная версия Rust ставится автоматически из `rust-toolchain.toml`;
- [just](https://just.systems), [cargo-nextest](https://nexte.st) и [cargo-deny](https://embarkstudios.github.io/cargo-deny/) (`cargo install --locked just cargo-nextest cargo-deny`);
- Docker с Compose, чтобы поднимать локальный ClickHouse;
- Node.js 24+ и yarn (`corepack enable`), только для сборки веб-интерфейса.

Все частые команды собраны в `Justfile`, их список выводит `just`:

```bash
# Поднять ClickHouse и запустить бэкенд с config.json
just run

# Собрать релизный бинарник со встроенным веб-интерфейсом
just build
./target/release/rustlog --config config.json
```

Веб-интерфейс встраивается в исполняемый файл только с feature `embed-frontend`. Без неё бэкенд собирается без Node.js, а вместо веб-интерфейса отдаётся страница-заглушка со ссылками на документацию API.

Если вы планируете отправлять изменения, запустите те же проверки, что и CI:

```bash
# Форматирование, clippy, тесты, аудит зависимостей и актуальность OpenAPI-спек
just check

# Integration-тесты HTTP API на локальном ClickHouse (нужен .env, как для docker-compose.dev.yml)
just test-integration

# Проверка типов веб-интерфейса
just web-check
```

### Тесты

Unit-тесты лежат рядом с кодом в `src/`, а integration-тесты HTTP API — в [`tests/http`](tests/http):

- `support/` — общий фикстур: отдельная база в ClickHouse с засеянными сообщениями, весь HTTP-стек поверх неё и построитель запросов;
- `legacy_*.rs`, `v2.rs`, `frontend_and_docs.rs` — по одному тесту на сценарий, сгруппированные по областям API;
- `snapshots/` — ожидаемые ответы (статус, заголовки, тело) для [insta](https://insta.rs), так что legacy API не может измениться незаметно.

Integration-тестам нужен ClickHouse, поэтому профиль nextest по умолчанию их не запускает (см. [`.config/nextest.toml`](.config/nextest.toml)): `just test` гоняет только unit-тесты, `just test-integration` — только integration. Чтобы запустить их на своём сервере ClickHouse (он должен работать в UTC), задайте `RUSTLOG_TEST_CLICKHOUSE_URL`, при необходимости `RUSTLOG_TEST_CLICKHOUSE_USER` и `RUSTLOG_TEST_CLICKHOUSE_PASSWORD`, и выполните `cargo nextest run --profile integration`. Обычный `cargo test` запустит и их, и без этих переменных они упадут с понятным сообщением.

Если поведение меняется намеренно, обновите snapshot-ы (`INSTA_UPDATE=always just test-integration`) и проверьте дифф в PR.

### OpenAPI

Спеки обоих API лежат в [`docs/openapi`](docs/openapi) и генерируются из кода командой `just openapi` (`rustlog openapi [каталог]`, ни конфиг, ни ClickHouse не нужны). CI проверяет, что закоммиченные файлы совпадают со сгенерированными, так что изменения API видны в диффе PR.

## Публикация Docker-образа

GitHub Actions публикует мультиархитектурный образ в [GHCR](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry) при отправке изменений вне PR:

```bash
docker pull ghcr.io/ruinateam/rustlog:main
docker run --rm -p 8026:8026 \
  -v "$PWD/config.json:/app/config.json:ro" \
  ghcr.io/ruinateam/rustlog:main
```

Порт публикуется по `listenAddress` из настроек. Секреты в образ не попадают, так как `.dockerignore` исключает `config.json`, а сетевые поля можно переопределить через переменные окружения.

## Конфигурация

Локальные (для каждого пользователя) настройки - `config.json`. Пример - `config.dist.json`, для описания полей смотрите [CONFIG.md](./docs/CONFIG.md).

```bash
./target/release/rustlog --config /path/to/config.json
```

Для контейнеров сетевые поля переопределяются переменными окружения - `RUSTLOG_CLICKHOUSE_URL`, `RUSTLOG_CLICKHOUSE_DB`, `RUSTLOG_CLICKHOUSE_USERNAME`, `RUSTLOG_CLICKHOUSE_PASSWORD`, `RUSTLOG_LISTEN_ADDRESS`, а формат логов — `RUSTLOG_LOG_FORMAT` (`text` или `json`). Настройки логирования, включая запись в файлы с ротацией, описаны в [CONFIG.md](./docs/CONFIG.md#поля) (секция `logging`).

Состояние каналов и opt-out после первой миграции хранится в ClickHouse. На один деплой должен быть только один бэкенд, который его меняет (смотрите [CONFIG.md](./docs/CONFIG.md#состояние-каналов-и-opt-out)).

## API и реал-тайм события

### API

- Старое - `/docs` и `/openapi.json`, спека в репозитории: [`docs/openapi/legacy.json`](docs/openapi/legacy.json)
- Новое (v2) - `/api/v2/docs` и `/api/v2/openapi.json`, спека в репозитории: [`docs/openapi/v2.json`](docs/openapi/v2.json), смотрите [API_V2.md](./docs/API_V2.md).
- `GET /metrics` — Prometheus-метрики.

### Реал-тайм события
- `GET /admin/firehose` — WebSocket эндпоинт, требует заголовок `X-Api-Key` (ключ нельзя передавать query-параметром), отдаёт сырой IRC без CRLF или `?format=json-basic`, отставший клиент закрывается кодом `1013`, а пропуск дочитывается через REST API.

## Tiers

Tiers отдаётся по эндпоинту `/{channel_id_type}/{channel}/tiers/{year}[/{month}[/{day}]]`.

- `channel_id_type` - тип канала (`channel` - логин, или `channelid` - Twitch ID)
- Периоды календарные, расчёт в `Europe/Moscow`, окна - фиксированные интервалы 1/5/15/30/60 минут.
- `?mode=all|online|offline` - все сообщения, только во время стримов или только вне стримов. Окна стримов берутся из [SullyGnome](https://sullygnome.com/) с откатом на кешированные значения.
- `?exclude_bots=<login,...>` - логины для исключения, по умолчанию вырезается набор распространённых ботов.
- При равных значениях порядок определяется по `user_id`, в ответе не больше 500 записей.
- Для `offline` уникальные сообщения считаются по фактическим сообщениям, а не как разность агрегатов.

## Импорт истории и обслуживание

### Зеркалирование определённого периода

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

### Удаление дублей Twitch-сообщений по ID

> [!WARNING]
> Перед использованием `--execute` делайте запасную копию и проверяйте dry-run.

```bash
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel>
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel> --execute
```

Без `--execute` команда только показывает отчёт. С ним она оставляет самую раннюю копию каждого id: откладывает её во временную таблицу, удаляет все копии мутацией и вставляет отложенные обратно. Остальные строки не затрагиваются. Мутацию команда ждёт `--wait-timeout` секунд (по умолчанию 600); если не дождалась, отложенные копии остаются во временной таблице, и ошибка называет её.

Перенос старых justlog-файлов и правила opt-out при импорте описаны в [MIGRATION.md](./docs/MIGRATION.md).

## CI
Workflow [`ci.yml`](.github/workflows/ci.yml) запускается на PR и на отправку изменений в `main`. Проверки идут параллельными задачами:

- форматирование и clippy;
- тесты через cargo-nextest и проверка актуальности OpenAPI-спек;
- integration-тесты HTTP API и команд импорта на ClickHouse в service-контейнере;
- аудит зависимостей через cargo-deny (уязвимости, лицензии, источники, настройки в [`deny.toml`](deny.toml));
- проверка типов и сборка веб-интерфейса, затем проверка бэкенда со встроенным веб-интерфейсом.

Docker-образ собирается только после успешных проверок. Для PR он собирается под `linux/amd64` и `linux/arm64` без публикации. На `main` образ с SBOM и provenance публикуется в [GHCR](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry) с тегами `main` и `sha-<commit>`, а при релизе ещё и с `X.Y.Z`, `X.Y` и `latest`.

Dependabot раз в неделю предлагает обновления Cargo-зависимостей и GitHub Actions.

## Релизы

Релизы ведёт [release-please](https://github.com/googleapis/release-please) по сообщениям коммитов в формате [Conventional Commits](https://www.conventionalcommits.org/ru/) (`feat:`, `fix:`, `refactor:` и т.д.):

1. После каждой отправки в `main` release-please открывает или обновляет PR с новой версией в `Cargo.toml` и записями в `CHANGELOG.md`.
2. Когда этот PR вливается, создаются тег `vX.Y.Z` и GitHub Release, а CI публикует образ с тегами версии.

Пока версия ниже `1.0.0`, `feat` и ломающие изменения поднимают minor-версию, а `fix` поднимает patch. Настройки лежат в [`.github/release-please-config.json`](.github/release-please-config.json).

> [!NOTE]
> Чтобы release-please мог открывать PR, в настройках репозитория (Settings → Actions → General → Workflow permissions) должна быть включена опция «Allow GitHub Actions to create and approve pull requests».
>
> PR от release-please создаётся с `GITHUB_TOKEN`, поэтому CI на нём не запускается. Если для `main` включены обязательные проверки, release-please нужно дать токен GitHub App или PAT.

## Лицензия

Этот проект распространяется под лицензией [MIT](LICENSE).
