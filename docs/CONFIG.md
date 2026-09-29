# Конфигурация

Rustlog читает настройки из `config.json` в корне проекта. Другой путь можно передать через `--config /path/to/config.json`.

Файл содержит доступы к ClickHouse, Twitch API и дополнительным интеграциям. Реальный `config.json` не должен попадать в git, потому что там есть секреты.

Для контейнерного запуска сетевые поля можно переопределить без копирования секретов в другой файл:

- `RUSTLOG_CLICKHOUSE_URL` переопределяет `clickhouseUrl`.
- `RUSTLOG_CLICKHOUSE_DB` переопределяет `clickhouseDb`.
- `RUSTLOG_CLICKHOUSE_USERNAME` переопределяет `clickhouseUsername`.
- `RUSTLOG_CLICKHOUSE_PASSWORD` переопределяет `clickhousePassword`.
- `RUSTLOG_LISTEN_ADDRESS` переопределяет `listenAddress`.

Development compose использует эти переменные для соединения с сервисом `clickhouse` и публикует rustlog на порту `8026`. Twitch credentials и admin keys остаются в `config.json`, смонтированном read-only.

## Минимальный пример

```json
{
  "clickhouseUrl": "http://localhost:8123",
  "clickhouseDb": "rustlog",
  "clickhouseUsername": "default",
  "clickhousePassword": "",
  "listenAddress": "0.0.0.0:8026",
  "logging": { "filter": "info", "format": "text" },
  "channels": ["44407373", "684505240"],
  "clientID": "twitch-client-id",
  "clientSecret": "twitch-client-secret",
  "admins": ["44407373"],
  "optOut": {},
  "adminAPIKey": "local-admin-key",
  "enableTierSnapshots": false
}
```

## Поля

`clickhouseUrl`

HTTP endpoint ClickHouse. Для локального WSL-запуска обычно:

```text
http://localhost:8123
```

`clickhouseDb`

Название базы ClickHouse. Обычно:

```text
rustlog
```

`clickhouseUsername` и `clickhousePassword`

Пользователь и пароль ClickHouse.

`clickhouseFlushInterval`

Раз во сколько секунд буфер новых сообщений записывается в ClickHouse; по умолчанию `10`. Сообщения из буфера видны в API и до записи.

`listenAddress`

Адрес backend HTTP API. В текущем локальном запуске используется:

```text
0.0.0.0:8026
```

`channels`

Начальный список Twitch channel id для миграции со старого конфига. Это именно числовые id, не логины каналов.

При первом старте текущие значения переносятся в ClickHouse. После миграции источником состояния становятся таблицы `channel_membership_state` и `opt_out_state`; команды join/part и opt-out больше не переписывают JSON-конфиг. Поле оставляют как снимок для аудита и аварийного отката.

`clientID` и `clientSecret`

Twitch application credentials. Нужны для Twitch API и IRC-подключения.

`admins`

Список Twitch user id, которым доступны admin-команды в чате: `!rustlog join <логин…>` и `!rustlog leave <логин…>` подключают и отключают каналы, а `!rustlog optout`, `optin`, `optout-channel` и `optin-channel` с логином вместо кода меняют opt-out любого пользователя или канала (смотрите [«Состояние каналов и opt-out»](#состояние-каналов-и-opt-out)).

`optOut`

Устаревший снимок пользователей или каналов, которых не нужно логировать. При первой миграции сохраняется наличие каждого ключа, включая значение `false`, так как старый runtime проверял наличие ключа, а не его boolean-значение. Такой ключ действует и на пользователя, и на канал с этим id (у стримера они совпадают); отмена opt-out одного из них не затрагивает другого.

```json
{
  "123456": true
}
```

`adminAPIKey`

Ключ для admin API: подключение и отключение каналов, opt-out каналов и пользователей и его отмена (`/api/v2/admin/...`, список — в [API_V2.md](./API_V2.md#admin)), а также старые `/admin/channels`. Без ключа admin API отвечает отказом. Передается только заголовком `X-Api-Key`; не добавляй ключ в query string, WebSocket subprotocol или URL, так как URI попадает в access logs.

Тот же ключ защищает `GET /admin/firehose`. Это WebSocket с живыми сообщениями, которые уже приняты очередью writer, но могут еще не быть записаны в ClickHouse. По умолчанию одно текстовое сообщение WebSocket содержит raw IRC без завершающего CRLF; `?format=json-basic` отправляет по одному basic JSON-объекту. Медленный клиент закрывается с кодом `1013`, после чего должен переподключиться и воспроизвести пропуск через HTTP API. Обычный browser WebSocket не умеет отправлять `X-Api-Key`, поэтому для браузерного доступа нужен отдельный same-site/proxy authentication слой, а не ключ в URL.

`supabaseUrl` и `supabaseServiceKey`

Опциональные поля для Supabase-интеграции. `supabaseServiceKey` является секретом.

`enableTierSnapshots`

Явное разрешение на экспорт tier-результатов во внешний Supabase RPC. По умолчанию `false`: наличие Supabase credentials само по себе не отправляет пользовательские данные во внешний сервис. Перед включением убедись, что для удаленных данных есть политика хранения и удаления после opt-out.

`logging`

Необязательная секция с настройками логов. Без неё логи уровня `info` и выше пишутся в stdout читаемым текстом.

```json
{
  "filter": "info",
  "format": "text",
  "file": {
    "directory": "logs",
    "maxFiles": 14
  }
}
```

- `filter` — какие события писать, в синтаксисе `RUST_LOG`: `info`, `debug`, `info,rustlog=debug` и т.п. Переменная окружения `RUST_LOG` важнее этого поля.
- `format` — `text` (читаемые строки с полями `key=value`) или `json` (один JSON-объект на событие, удобно для `jq` и сборщиков логов). Переопределяется переменной `RUSTLOG_LOG_FORMAT`.
- `file` — дополнительно писать те же события в файлы `rustlog.YYYY-MM-DD.log` в каталоге `directory` с ежедневной ротацией; хранятся последние `maxFiles` файлов (по умолчанию 14). Без этой секции логи идут только в stdout.

Цвета в текстовом формате включаются, только если stdout — терминал и не задана переменная `NO_COLOR`; `RUST_LOG_ANSI=true|false` включает или выключает их принудительно. Паники тоже попадают в лог с сообщением и местом в коде.

## Состояние каналов и opt-out

Миграция `8_channel_membership_and_opt_out_state` создаёт две ClickHouse-таблицы состояния и переносит значения из активного конфигурационного файла. Миграция идемпотентна: повторный запуск записывает одинаковое начальное состояние и не меняет JSON.

Каждое изменение opt-out — новая строка с большей ревизией, действует последняя. Opt-out бывает двух видов:

- **Пользователь** (`!rustlog optout <код>`, `PUT /api/v2/admin/users/{id}/opt-out`): его сообщения перестают записываться, уже записанные сообщения и история ников удаляются. Удаление выполняется ClickHouse mutation асинхронно; до его завершения запросы отфильтровывают строки такого пользователя. Отмена (`!rustlog optin <код>`, `DELETE …/opt-out`) снова включает запись, но удалённое не возвращает.
- **Канал** (`!rustlog optout-channel <код>` в собственном чате от аккаунта стримера, `PUT /api/v2/admin/channels/{id}/opt-out`): запись канала прекращается, его данные отвечают `403` и пропадают из списков, но строки в ClickHouse остаются. Бот остаётся в чате, чтобы принять команду отмены (`!rustlog optin-channel <код>`, `DELETE …/opt-out`), после которой видна и старая история.

Код выдаёт `POST /api/v2/opt-out-codes` (веб-интерфейс — страница `/opt-out`). Коды одноразовые, живут минуту и хранятся в памяти процесса: после перезапуска выданные коды недействительны.

Opt-out применяется до записи live-сообщения, к очереди writer, историческим импортам и чтениям. Ответы с историей, доступными периодами, статистикой и tiers отдаются с `Cache-Control: no-cache`, чтобы кеши перепроверяли политику доступа.

Текущая реализация рассчитана на один активный backend-процесс, который меняет состояние. Revision формируется внутри процесса, а коды opt-out хранятся в его памяти, поэтому несколько одновременно работающих инстансов требуют внешней блокировки или другого транзакционного control-plane хранилища.

Старые Supabase tier snapshots не удаляются автоматически: текущий RPC умеет только upsert. После opt-out пользователя оператор должен удалить исторические snapshots отдельно, прежде чем считать удаление данных завершенным.

## Проверка конфига

После изменения `config.json` перезапусти backend:

```bash
sudo systemctl restart rustlog.service
journalctl -u rustlog.service -n 50 --no-pager
```

Если конфиг некорректный, сервис обычно падает на старте и ошибка будет в journal.
