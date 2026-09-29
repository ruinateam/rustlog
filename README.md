# ChatTiers Rustlog

Форк [rustlog](https://github.com/boring-nick/rustlog) — селф-хост логгер Twitch-чата. Пишет сообщения в ClickHouse, отдаёт их по REST API и в веб-интерфейсе, транслирует в реальном времени.

Отличия от [rustlog](https://github.com/boring-nick/rustlog):

- **Tiers** — рейтинг чаттеров по фиксированным окнам 1/5/15/30/60 минут с тирами HT1..LT5 и календарными периодами в `Europe/Moscow`.
- **API v2** — всё, что умеет старое API, с едиными соглашениями: Twitch ID в путях, строгие диапазоны RFC 3339, ошибки `application/problem+json` (RFC 9457). Старое justlog-совместимое API работает без изменений.
- **Новый веб-интерфейс** — Vue 3 на API v2: логи с эмоутами и значками, тиры, статистика, opt-out; русский и английский языки, светлая и тёмная тема.
- **Opt-out пользователей и каналов** — самостоятельно командой в чате или через admin API, с возможностью отмены.
- **Admin Firehose** — WebSocket API для сообщений в реальном времени.
- **Состояние в ClickHouse** — подключённые каналы и opt-out живут в таблицах, а не в JSON-конфиге.
- **Импорт истории** — `mirror`, `fill-missing`, `migrate` и `cleanup-duplicate-ids` с соблюдением opt-out.

## Быстрый старт

Для старта понадобится [Docker](https://www.docker.com/get-started):

```bash
# Склонируйте и перейдите в директорию
git clone https://github.com/ruinateam/rustlog.git
cd rustlog

# Скопируйте настройки и заполните поля
cp .env.example .env
cp config.dist.json config.json

# Запустите контейнеры
docker compose -f docker-compose.dev.yml up -d
docker compose -f docker-compose.dev.yml ps
```

После старта доступны:

- Веб-интерфейс и API — [localhost:8026](http://localhost:8026)
- Документация API — [localhost:8026/docs](http://localhost:8026/docs)
- ClickHouse — [localhost:8123](http://localhost:8123)

Docker поднимает ClickHouse `26.5.6.64` с healthcheck-ом, монтирует `config.json` в бэкенд в режиме read-only, а сетевые поля настроек переопределяет через `RUSTLOG_*`. Данные от Twitch остаются в `config.json`.

## Локальная сборка

Требования:

- [rustup](https://rustup.rs): нужная версия Rust ставится автоматически из `rust-toolchain.toml`;
- [just](https://just.systems), [cargo-nextest](https://nexte.st) и [cargo-deny](https://embarkstudios.github.io/cargo-deny/) (`cargo install --locked just cargo-nextest cargo-deny`);
- Docker с Compose, чтобы поднимать локальный ClickHouse;
- [bun](https://bun.sh) (версия указана в `packageManager` в `web/package.json`) — только для веб-интерфейса.

Все частые команды собраны в `Justfile`, их список выводит `just`:

```bash
# Поднять ClickHouse и запустить бэкенд с config.json
just run

# Собрать релизный бинарник со встроенным веб-интерфейсом
just build
./target/release/rustlog --config config.json
```

Веб-интерфейс встраивается в исполняемый файл только с feature `embed-frontend`. Без неё бэкенд собирается без bun, а вместо веб-интерфейса отдаётся страница-заглушка со ссылками на документацию API.

Перед отправкой изменений запустите те же проверки, что и CI:

```bash
# Форматирование, clippy, unit-тесты, аудит зависимостей и актуальность OpenAPI-спек
just check

# Integration-тесты на локальном ClickHouse (нужен .env, как для docker-compose.dev.yml)
just test-integration

# Линтер, проверка типов и тесты веб-интерфейса, актуальность его API-типов
just web-check
```

### Веб-интерфейс

Лежит в [`web/`](web): Vue 3, Vite, TypeScript, Tailwind CSS и компоненты [shadcn-vue](https://www.shadcn-vue.com) (в `web/src/components/ui`, правятся как свои), данные — через TanStack Query, интерфейс на русском и английском. Он работает только с API v2: типы запросов и ответов генерируются из [`docs/openapi/v2.json`](docs/openapi/v2.json) в `web/src/api/schema.d.ts` командой `just web-api-types`, и CI проверяет, что они не устарели.

```bash
# Dev-сервер с горячей перезагрузкой; API проксируется на бэкенд с localhost:8025
# (другой адрес задаёт RUSTLOG_BACKEND_URL)
just web-dev
```

Страницы: главная со списком каналов, логи (`/logs`), тиры (`/tiers`) и opt-out (`/opt-out`). Пути состоят из одного сегмента, а состояние хранится в query (`/logs?channel=…&date=…`): более длинные пути занимает старое API (`/{channel_id_type}/{channel}` и т. д.).

### Тесты

- Unit-тесты бэкенда лежат рядом с кодом в `src/`, веб-интерфейса — рядом с кодом в `web/src` (`*.test.ts`, [Vitest](https://vitest.dev)).
- Integration-тесты HTTP API — в [`tests/http`](tests/http): `support/` поднимает отдельную базу в ClickHouse с засеянными сообщениями и весь HTTP-стек поверх неё; `legacy_*.rs`, `v2_*.rs` и `frontend_and_docs.rs` сгруппированы по областям API; `snapshots/` хранит ожидаемые ответы (статус, заголовки, тело) для [insta](https://insta.rs), так что ни одно API не может измениться незаметно.
- Integration-тесты служебных команд — в [`tests/tools`](tests/tools): импорт из поддельного зеркала на локальном HTTP-сервере, из кеша и из файлов justlog, дедупликация.

Integration-тестам нужен ClickHouse, поэтому профиль nextest по умолчанию их не запускает (см. [`.config/nextest.toml`](.config/nextest.toml)): `just test` гоняет только unit-тесты, `just test-integration` — только integration. Чтобы запустить их на своём ClickHouse (он должен работать в UTC), задайте `RUSTLOG_TEST_CLICKHOUSE_URL`, при необходимости `RUSTLOG_TEST_CLICKHOUSE_USER` и `RUSTLOG_TEST_CLICKHOUSE_PASSWORD`, и выполните `cargo nextest run --profile integration`.

Если поведение меняется намеренно, обновите snapshot-ы (`INSTA_UPDATE=always just test-integration`) и проверьте дифф в PR.

### OpenAPI

Спеки обоих API лежат в [`docs/openapi`](docs/openapi) и генерируются из кода командой `just openapi` (`rustlog openapi [каталог]`, ни конфиг, ни ClickHouse не нужны). CI проверяет, что закоммиченные файлы совпадают со сгенерированными, так что изменения API видны в диффе PR.

## Конфигурация

Настройки — `config.json`, пример — `config.dist.json`, описание полей — в [CONFIG.md](./docs/CONFIG.md).

```bash
./target/release/rustlog --config /path/to/config.json
```

Для контейнеров сетевые поля переопределяются переменными окружения `RUSTLOG_CLICKHOUSE_URL`, `RUSTLOG_CLICKHOUSE_DB`, `RUSTLOG_CLICKHOUSE_USERNAME`, `RUSTLOG_CLICKHOUSE_PASSWORD`, `RUSTLOG_LISTEN_ADDRESS`, а формат логов — `RUSTLOG_LOG_FORMAT` (`text` или `json`). Настройки логирования, включая запись в файлы с ротацией, описаны в [CONFIG.md](./docs/CONFIG.md#поля) (секция `logging`).

Состояние каналов и opt-out после первой миграции хранится в ClickHouse. На один деплой должен быть только один бэкенд, который его меняет (смотрите [CONFIG.md](./docs/CONFIG.md#состояние-каналов-и-opt-out)).

## API

Документация обоих API — на одной странице `/docs`: по умолчанию v2, старое API выбирается в переключателе документов слева вверху (или `/docs?api=legacy`).

- **v2** — `/api/v2`, спека `/api/v2/openapi.json`, в репозитории — [`docs/openapi/v2.json`](docs/openapi/v2.json). Соглашения и все эндпоинты описаны в [API_V2.md](./docs/API_V2.md).
- **Старое** (justlog-совместимое, устаревшее) — в корне, спека `/openapi.json`, в репозитории — [`docs/openapi/legacy.json`](docs/openapi/legacy.json). Оно заморожено: работает как раньше для существующих клиентов, новые возможности появляются только в v2.
- `GET /metrics` — Prometheus-метрики.

### Реал-тайм события

`GET /admin/firehose` — WebSocket, требует заголовок `X-Api-Key` (ключ нельзя передавать query-параметром). Отдаёт сырой IRC без CRLF или, с `?format=json-basic`, basic JSON. Отставший клиент закрывается кодом `1013`, а пропуск дочитывается через REST API. Подробнее — в [CONFIG.md](./docs/CONFIG.md).

### Tiers

`GET /api/v2/channels/{channelId}/tiers/{period}`, где `period` — день (`2026-03-01`), месяц (`2026-03`) или год (`2026`). В старом API — `/{channel_id_type}/{channel}/tiers/{year}[/{month}[/{day}]]`.

- Периоды календарные, расчёт в `Europe/Moscow`, окна — фиксированные интервалы 1/5/15/30/60 минут.
- `?mode=all|online|offline` — все сообщения, только во время стримов или только вне стримов. Окна стримов берутся из [SullyGnome](https://sullygnome.com/) с откатом на кешированные значения.
- `?excludeBots=<логин>` (повторяемый; в старом API `?exclude_bots=<логин,…>`) — кого не учитывать, по умолчанию вырезается набор распространённых ботов.
- При равных значениях порядок определяется по `user_id`, в ответе не больше 500 записей.
- Для `offline` уникальные сообщения считаются по фактическим сообщениям, а не как разность агрегатов.

## Opt-out

Пользователи и каналы отказываются от логирования и отменяют отказ командой в чате. Аккаунт на стороне rustlog не нужен: Twitch подтверждает, кто отправил сообщение.

1. Получите одноразовый код на странице `/opt-out` веб-интерфейса или запросом `POST /api/v2/opt-out-codes`. Код действует минуту.
2. Напишите команду с кодом в чате Twitch:

| Команда | Где | Что делает |
| --- | --- | --- |
| `!rustlog optout <код>` | в любом логируемом чате | перестаёт логировать отправителя и удаляет его сообщения и логины во всех каналах |
| `!rustlog optin <код>` | в любом логируемом чате | снова логирует отправителя; удалённое не возвращается |
| `!rustlog optout-channel <код>` | в своём чате, от аккаунта стримера | перестаёт логировать канал и скрывает его логи, но не удаляет их |
| `!rustlog optin-channel <код>` | в своём чате, от аккаунта стримера | снова логирует канал и показывает его логи, включая старые |

Админы из `admins` конфига могут писать те же команды с логином вместо кода. Через admin API то же делают `PUT` и `DELETE` на `/api/v2/admin/users/{id}/opt-out` и `/api/v2/admin/channels/{id}/opt-out` с заголовком `X-Api-Key`. Подробности — в [CONFIG.md](./docs/CONFIG.md#состояние-каналов-и-opt-out).

## Импорт истории и обслуживание

Все команды описаны в [MIGRATION.md](./docs/MIGRATION.md), а их флаги выводит `rustlog <команда> --help`.

```bash
# Зеркалирование периода из rustlog/justlog API
./target/release/rustlog mirror --base-url https://logs.zonian.dev --channel <channel> --year 2026 --month 1

# Дозагрузка недостающих дней по зеркалам из https://logs.zonian.dev/api/<channel>
./target/release/rustlog fill-missing --year 2026 --channel <channel> --dry-run
./target/release/rustlog fill-missing --year 2026 --channel <channel>

# Импорт файлов justlog
./target/release/rustlog migrate --source-dir /path/to/justlog/logs
```

Полезные флаги: `--local-cache`, `--proxy` (повторяемый), `--rps`, `--http-concurrency`, `--exclude-instance`; для `fill-missing` — `--repair-existing` и `--deep`. Повторный `mirror` не дублирует уже сохранённые сообщения.

### Удаление дублей Twitch-сообщений по ID

> [!WARNING]
> Перед использованием `--execute` делайте запасную копию и проверяйте dry-run.

```bash
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel>
./target/release/rustlog cleanup-duplicate-ids --year 2026 --channel <channel> --execute
```

Без `--execute` команда только показывает отчёт. С ним она оставляет самую раннюю копию каждого id: откладывает её во временную таблицу, удаляет все копии мутацией и вставляет отложенные обратно. Остальные строки не затрагиваются. Мутацию команда ждёт `--wait-timeout` секунд (по умолчанию 600); если не дождалась, отложенные копии остаются во временной таблице, и ошибка называет её.

## Публикация Docker-образа

GitHub Actions публикует мультиархитектурный образ в [GHCR](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry) при отправке изменений вне PR:

```bash
docker pull ghcr.io/ruinateam/rustlog:main
docker run --rm -p 8026:8026 \
  -v "$PWD/config.json:/app/config.json:ro" \
  ghcr.io/ruinateam/rustlog:main
```

Порт публикуется по `listenAddress` из настроек. Секреты в образ не попадают, так как `.dockerignore` исключает `config.json`, а сетевые поля можно переопределить через переменные окружения.

## CI

Workflow [`ci.yml`](.github/workflows/ci.yml) запускается на PR и на отправку изменений в `main`. Проверки идут параллельными задачами:

- форматирование и clippy;
- unit-тесты через cargo-nextest и проверка актуальности OpenAPI-спек;
- integration-тесты HTTP API и служебных команд на ClickHouse в service-контейнере;
- аудит зависимостей через cargo-deny (уязвимости, лицензии, источники, настройки в [`deny.toml`](deny.toml));
- веб-интерфейс: актуальность API-типов, линтер, проверка типов, тесты и сборка, затем проверка бэкенда со встроенным веб-интерфейсом.

Docker-образ собирается только после успешных проверок. Для PR он собирается под `linux/amd64` и `linux/arm64` без публикации. На `main` образ с SBOM и provenance публикуется в GHCR с тегами `main` и `sha-<commit>`, а при релизе ещё и с `X.Y.Z`, `X.Y` и `latest`.

Dependabot раз в неделю предлагает обновления Cargo-зависимостей, пакетов веб-интерфейса и GitHub Actions.

## Релизы

Релизы ведёт [release-please](https://github.com/googleapis/release-please) по сообщениям коммитов в формате [Conventional Commits](https://www.conventionalcommits.org/ru/) (`feat:`, `fix:`, `refactor:` и т.д.):

1. После каждой отправки в `main` release-please открывает или обновляет PR с новой версией в `Cargo.toml` и OpenAPI-спеках и записями в `CHANGELOG.md`.
2. Когда этот PR вливается, создаются тег `vX.Y.Z` и GitHub Release, а CI публикует образ с тегами версии.

Пока версия ниже `1.0.0`, `feat` и ломающие изменения поднимают minor-версию, а `fix` поднимает patch. Настройки лежат в [`.github/release-please-config.json`](.github/release-please-config.json).

> [!NOTE]
> Чтобы release-please мог открывать PR, в настройках репозитория (Settings → Actions → General → Workflow permissions) должна быть включена опция «Allow GitHub Actions to create and approve pull requests».
>
> PR от release-please создаётся с `GITHUB_TOKEN`, поэтому CI на нём не запускается. Если для `main` включены обязательные проверки, release-please нужно дать токен GitHub App или PAT.

## Лицензия

Этот проект распространяется под лицензией [MIT](LICENSE).
