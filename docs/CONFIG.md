# Конфигурация

Rustlog читает настройки из `config.json` в корне проекта.

Файл содержит доступы к ClickHouse, Twitch API и дополнительным интеграциям. Реальный `config.json` не должен попадать в git, потому что там есть секреты.

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
  "adminAPIKey": "local-admin-key"
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

Список Twitch channel id, которые бот логирует в live-режиме. Это именно числовые id, не логины каналов.

`clientID` и `clientSecret`

Twitch application credentials. Нужны для Twitch API и IRC-подключения.

`admins`

Список Twitch user id, которым доступны admin-команды.

`optOut`

Словарь пользователей, которых не нужно логировать.

```json
{
  "123456": true
}
```

`adminAPIKey`

Ключ для защищенных admin API-запросов, если они используются.

`supabaseUrl` и `supabaseServiceKey`

Опциональные поля для Supabase-интеграции. `supabaseServiceKey` является секретом.

## Проверка конфига

После изменения `config.json` перезапусти backend:

```bash
sudo systemctl restart rustlog.service
journalctl -u rustlog.service -n 50 --no-pager
```

Если конфиг некорректный, сервис обычно падает на старте и ошибка будет в journal.
