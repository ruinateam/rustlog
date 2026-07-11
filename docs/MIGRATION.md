# Миграция и исторический импорт

Rustlog поддерживает два сценария переноса истории:

1. Миграция локальных файлов justlog.
2. Зеркалирование истории из удаленных rustlog/justlog API.

Для текущего локального ChatTiers-стека основной способ - зеркала через `mirror` и `fill-missing`.

## Миграция локальных justlog-файлов

Если есть папка со старыми файлами justlog, ее можно импортировать напрямую в ClickHouse.

```bash
./target/release/rustlog migrate \
  --source-dir /path/to/justlog/logs \
  --jobs 1
```

`--jobs` задает число параллельных потоков. Для HDD лучше оставить `1`; для SSD можно увеличить, но ClickHouse и диск все равно будут основным лимитом.

## Зеркалирование из API

Команда `mirror` импортирует JSON-логи из rustlog/justlog API.

```bash
./target/release/rustlog mirror \
  --base-url https://logs.zonian.dev \
  --channel zakvielchannel \
  --year 2026 \
  --month 1 \
  --batch 1000
```

Можно ограничить импорт конкретным днем:

```bash
./target/release/rustlog mirror \
  --base-url https://logs.zonian.dev \
  --channel zakvielchannel \
  --year 2026 \
  --month 1 \
  --day 2
```

Если нужен прокси из WSL, используй Windows-host адрес:

```bash
./target/release/rustlog mirror \
  --base-url https://logs.zonian.dev \
  --channel zakvielchannel \
  --year 2026 \
  --proxy http://172.30.96.1:10808
```

## Импорт из локального cache

Если JSON уже скачан локально, можно импортировать без HTTP.

Ожидаемая структура:

```text
cache/<channel>/daily/YYYY/MM/DD.json
```

Команда:

```bash
./target/release/rustlog mirror \
  --local-cache /mnt/c/Users/Linar/Desktop/twitchlogs/cache \
  --channel zakvielchannel \
  --year 2026
```

## Дозагрузка только пропущенных дней

`fill-missing` берет список зеркал из:

```text
https://logs.zonian.dev/api/<channel>
```

Затем сравнивает доступные дни с локальными днями в ClickHouse и импортирует только отсутствующие.

Dry-run:

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

## Дедупликация после импорта

Во время повторных импортов могут появиться дубли. Проверка идет по Twitch message id.

Проверить:

```bash
./target/release/rustlog cleanup-duplicate-ids \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney
```

Удалить дубли:

```bash
./target/release/rustlog cleanup-duplicate-ids \
  --year 2026 \
  --channel linaryx \
  --channel zakvielchannel \
  --channel jacklooney \
  --execute
```

Без `--execute` команда ничего не меняет. С `--execute` она оставляет одну строку на `(channel_login, id)` и удаляет остальные копии.

## Проверка результата

После импорта проверь API/backend и состояние сервиса:

```bash
systemctl status rustlog.service --no-pager -l
curl -I http://localhost:8026
```

Для просмотра ошибок:

```bash
journalctl -u rustlog.service -n 100 --no-pager
```
