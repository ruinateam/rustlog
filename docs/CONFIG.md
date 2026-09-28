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

Список Twitch user id, которым доступны admin-команды.

`optOut`

Устаревший снимок пользователей или каналов, которых не нужно логировать. При первой миграции сохраняется наличие каждого ключа, включая значение `false`, так как старый runtime проверял наличие ключа, а не его boolean-значение.

```json
{
  "123456": true
}
```

`adminAPIKey`

Ключ для защищенных admin API-запросов, если они используются. Передается только заголовком `X-Api-Key`; не добавляй ключ в query string, WebSocket subprotocol или URL, так как URI попадает в access logs.

Тот же ключ защищает `GET /admin/firehose`. Это WebSocket с живыми сообщениями, которые уже приняты очередью writer, но могут еще не быть записаны в ClickHouse. По умолчанию одно текстовое сообщение WebSocket содержит raw IRC без завершающего CRLF; `?format=json-basic` отправляет по одному basic JSON-объекту. Медленный клиент закрывается с кодом `1013`, после чего должен переподключиться и воспроизвести пропуск через HTTP API. Обычный browser WebSocket не умеет отправлять `X-Api-Key`, поэтому для браузерного доступа нужен отдельный same-site/proxy authentication слой, а не ключ в URL.

`supabaseUrl` и `supabaseServiceKey`

Опциональные поля для Supabase-интеграции. `supabaseServiceKey` является секретом.

`enableTierSnapshots`

Явное разрешение на экспорт tier-результатов во внешний Supabase RPC. По умолчанию `false`: наличие Supabase credentials само по себе не отправляет пользовательские данные во внешний сервис. Перед включением убедись, что для удаленных данных есть политика хранения и удаления после opt-out.

## Состояние каналов и opt-out

Миграция `8_channel_membership_and_opt_out_state` создаёт две ClickHouse-таблицы состояния и переносит значения из активного конфигурационного файла. Миграция идемпотентна: повторный запуск записывает одинаковое начальное состояние и не меняет JSON.

Opt-out применяется до записи live-сообщения, к очереди writer, историческим импортам и публичным чтениям. Удаление уже записанных строк выполняется ClickHouse mutation асинхронно; до его завершения API отфильтровывает opt-out пользователей в запросах. Ответы с историей, доступными периодами и tiers отдаются с `Cache-Control: no-cache`, чтобы кеши перепроверяли политику доступа.

Текущая реализация рассчитана на один активный backend-процесс, который меняет состояние. Revision формируется внутри процесса, поэтому несколько одновременно пишущих инстансов требуют внешней блокировки или другого транзакционного control-plane хранилища.

Старые Supabase tier snapshots не удаляются автоматически: текущий RPC умеет только upsert. После включения opt-out оператор должен удалить исторические snapshots отдельно, прежде чем считать удаление данных завершенным.

## Проверка конфига

После изменения `config.json` перезапусти backend:

```bash
sudo systemctl restart rustlog.service
journalctl -u rustlog.service -n 50 --no-pager
```

Если конфиг некорректный, сервис обычно падает на старте и ошибка будет в journal.
