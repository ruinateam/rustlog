set dotenv-load

compose := "docker compose -f docker-compose.dev.yml"

# List available recipes
_default:
    @just --list

# Run the backend against the local ClickHouse
run *args: db-up
    cargo run -- {{ args }}

# Run the same checks as CI
check: fmt-check clippy test deny openapi-check

# Format the code
fmt:
    cargo fmt

# Check formatting without changing files
fmt-check:
    cargo fmt -- --check

# Lint the code
clippy:
    cargo clippy --all-targets -- -D warnings

# Run the tests
test *args:
    cargo nextest run {{ args }}

# Run the integration tests against the local ClickHouse (needs .env)
test-integration *args: db-up
    RUSTLOG_TEST_CLICKHOUSE_URL=http://localhost:8123 \
    RUSTLOG_TEST_CLICKHOUSE_USER="${CLICKHOUSE_USER:-user}" \
    RUSTLOG_TEST_CLICKHOUSE_PASSWORD="${CLICKHOUSE_PASSWORD:-}" \
    cargo nextest run --profile integration {{ args }}

# Audit dependencies for advisories, licenses and banned crates
deny:
    cargo deny check

# Regenerate the committed OpenAPI documents in docs/openapi
openapi:
    RUST_LOG=warn cargo run -q -- openapi

# Fail if the committed OpenAPI documents are out of date
openapi-check: openapi
    git diff --exit-code -- docs/openapi
    test -z "$(git status --porcelain -- docs/openapi)"

# Run the web frontend with hot reload against the local backend
web-dev:
    cd web && bun install --frozen-lockfile && bun run dev

# Build the web frontend into web/dist
web-build:
    cd web && bun install --frozen-lockfile && bun run build

# Lint, typecheck and test the web frontend
web-check: web-api-types-check
    cd web && bun install --frozen-lockfile && bun run check

# Regenerate the API types of the web frontend from the v2 OpenAPI document
web-api-types:
    cd web && bun run api-types

# Fail if the API types of the web frontend are out of date
web-api-types-check: web-api-types
    git diff --exit-code -- web/src/api/schema.d.ts

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
