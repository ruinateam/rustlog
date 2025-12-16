# Rustlog

## Description
Rustlog is a Twitch logging service based on [justlog](https://github.com/gempir/justlog). It provides the same web UI and API, but it utilizes [Clickhouse](https://clickhouse.com) for storage instead of text files.


## Installation

Create a `config.json` file (see [CONFIG.md](./docs/CONFIG.md))

### Docker
```yaml
version: "3.8"
  
services:
  clickhouse:
    image: clickhouse/clickhouse-server:latest
    container_name: clickhouse
    volumes:
      - "./ch-data:/var/lib/clickhouse:rw"
    environment:
      CLICKHOUSE_DB: "rustlog"
      CLICKHOUSE_USER: "user"
      CLICKHOUSE_PASSWORD: "SuperSecretPassword"
    restart: unless-stopped
        
  rustlog:
    image: ghcr.io/boring-nick/rustlog:master
    container_name: rustlog
    ports:
      - 8025:8025 
    volumes:
      - "./config.json:/config.json"
    depends_on: 
      - clickhouse
    restart: unless-stopped
```

### From source

- Follow the [Contributing](Contributing) excluding the last step
- `cargo build --release`
- The resulting binary will be at `target/release/rustlog`

## Advantages over justlog

- Significantly better storage efficiency (3x+ improvement) thanks to not duplicating log files, more efficient structure and better compression (using ZSTD in Clickhouse)
- Blazing fast log queries with response streaming and a [highly performant IRC parser](https://github.com/jprochazk/twitch-rs)
- Support for ndjson logs responses

## Contributing

Requirements:
- rust
- yarn
- docker with docker-compose (optional, will need to set up Clickhouse manually without it)

Steps:

0. Clone the repository (make sure to include submodules!):
```
git clone --recursive https://github.com/boring-nick/rustlog
```
If you already cloned the repo without `--recursive`, you can initialize submodules with:
```
git submodule update --init --recursive
```

1. Set up the database (Clickhouse):

This repository provides a docker-compose to quickly set up Clickhouse. You can use it with:
```
docker-compose -f docker-compose.dev.yml up -d
```
Alternatively, you can install Clickhouse manually using the [official guide](https://clickhouse.com/docs/en/install).

2. Create a config file

Copy `config.dist.json` to `config.json` and configure your database and twitch credentials. If you installed Clickhouse with Docker, the default database configuration works.

3. Build the frontend:
```
cd web
yarn install
yarn build
cd ..
```
4. Build and run rustlog:
```
cargo run
```

You can now access rustlog at http://localhost:8025.

## Migrating from justlog
See [MIGRATION.md](./docs/MIGRATION.md)

## Mirroring existing logs into ClickHouse

Если на VPS нет своих логов, можно подтянуть их с чужого rustlog/justlog (или из локального кэша) и записать в ClickHouse.

Пример команд:
```bash
# 1) HTTP-зеркалирование с удалённого rustlog
cargo run --release -- mirror \
  --base-url https://logs.zonian.dev \
  --channel zakvielchannel \
  --year 2025 \
  --month 12 \
  --batch 1000

# 2) Использовать локальный кэш вместо HTTP (структура cache/<channel>/daily/YYYY/MM/DD.json)
cargo run --release -- mirror \
  --local-cache /mnt/c/Users/Linar/Desktop/twitchlogs/cache \
  --channel zakvielchannel \
  --year 2025
```
Параметры:
- `--channel` (обязательно) – логин канала.
- `--base-url` – корень API rustlog/justlog (по умолчанию https://logs.zonian.dev).
- `--local-cache` – путь к кэшу JSON (если указан, HTTP не используется).
- `--year/--month/--day` – фильтры периода (опционально).
- `--batch` – размер вставки в ClickHouse (по умолчанию 1000).

После зеркалирования API отдаёт логи/tiers как будто они собраны локально. 
