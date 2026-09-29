# Миграция и исторический импорт

Rustlog поддерживает два сценария переноса истории:

1. Миграция локальных файлов justlog.
2. Зеркалирование истории из удаленных rustlog/justlog API.

Для текущего локального ChatTiers-стека основной способ - зеркала через `mirror` и `fill-missing`.

## Политика opt-out при импорте

Перед импортом Rustlog загружает сохраненное состояние каналов и opt-out из ClickHouse. Исторические строки для opted-out пользователей и каналов не добавляются, даже если они присутствуют в локальном файле или удаленном зеркале.

Состояние загружается один раз при старте команды. Если кто-то откажется от логирования, пока идёт долгий импорт, его сообщения из этого импорта всё равно будут записаны. Читать их API не даст: запросы исключают строки opted-out пользователей, а данные opted-out каналов отвечают `403`. Чтобы удалить и сами строки, повторите opt-out пользователя после импорта (`PUT /api/v2/admin/users/{id}/opt-out`).

Это не заменяет первоначальную миграцию состояния: сначала запусти backend с актуальным конфигом, чтобы он выполнил `8_channel_membership_and_opt_out_state`. Не удаляй `channels` и `optOut` из старого JSON до успешного запуска - они являются исходным снимком для этой миграции.

## Миграция локальных justlog-файлов

Если есть папка со старыми файлами justlog, ее можно импортировать напрямую в ClickHouse.

```bash
./target/release/rustlog migrate \
  --source-dir /path/to/justlog/logs \
  --jobs 1
```

Ожидаемая структура — как у justlog: `<id канала>/<год>/<месяц>/<день>/channel.txt` или `channel.txt.gz` (если есть оба, читается сжатый). `--channel-id` (`-c`, повторяемый) ограничивает импорт указанными каналами, без него импортируются все папки.

`--jobs` задает, сколько месяцев импортируется параллельно. Для HDD лучше оставить `1`; для SSD можно увеличить, но ClickHouse и диск все равно будут основным лимитом. Если папки нет, команда завершается ошибкой; папки с неожиданными именами пропускаются с предупреждением в логе.

## Зеркалирование из API

Команда `mirror` импортирует JSON-логи из rustlog/justlog API. Сообщения без Twitch message id и с id, которые уже есть в ClickHouse за этот период, пропускаются, поэтому повторный запуск ничего не дублирует. Дни, которые не удалось скачать, повторяются один раз, затем пропускаются с предупреждением в логе.

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

Затем сравнивает доступные дни с локальными днями в ClickHouse и импортирует только отсутствующие, перебирая зеркала по очереди, пока все дни не найдутся. Если какой-то день не нашёлся ни на одном зеркале, команда завершается ошибкой со списком таких дней.

С `--repair-existing` она ещё и сравнивает уже сохранённые дни с зеркалом по числу уникальных message id и докачивает дни, где у зеркала сообщений больше; `--deep` сравнивает со всеми зеркалами, а не с первым ответившим. `--exclude-instance` (повторяемый) исключает зеркало.

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

Без `--execute` команда ничего не меняет и только показывает отчёт. С `--execute` она оставляет самую раннюю копию каждого id: откладывает её во временную таблицу, удаляет все копии мутацией, ждёт её до `--wait-timeout` секунд и вставляет отложенные копии обратно. Строки вне `--channel` и `--year` и сообщения без id не затрагиваются. Если мутация не успела, отложенные копии остаются во временной таблице, и ошибка называет её. Перед любым `--execute` сделай резервную копию ClickHouse и проверь dry-run: операция удаляет данные и не должна запускаться как регулярная необслуживаемая задача.

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
