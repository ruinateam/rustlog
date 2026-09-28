compose := "docker compose -f docker-compose.dev.yml"

# List available recipes
_default:
    @just --list

# Run the backend against the local ClickHouse
run *args: db-up
    cargo run -- {{ args }}

# Run the same checks as CI
check: fmt-check clippy test deny

# Format the code
fmt:
    cargo fmt

# Check formatting without changing files
fmt-check:
    cargo fmt -- --check

# Lint the code
clippy:
    cargo clippy --all-targets

# Run the tests
test *args:
    cargo nextest run {{ args }}

# Audit dependencies for advisories, licenses and banned crates
deny:
    cargo deny check

# Build the web frontend into web/dist
web-build:
    cd web && yarn install --frozen-lockfile --ignore-optional && yarn build

# Typecheck the web frontend
web-check:
    cd web && yarn install --frozen-lockfile --ignore-optional && yarn typecheck

# Build a release binary with the embedded frontend
build: web-build
    cargo build --release --features embed-frontend

# Start the local ClickHouse in the background
db-up:
    {{ compose }} up -d --wait clickhouse

# Stop the local ClickHouse
db-down:
    {{ compose }} down

# Build the Docker image locally
docker-build tag="rustlog:dev":
    docker build -t {{ tag }} .
