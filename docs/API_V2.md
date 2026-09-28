# API v2

API v2 is additive. Legacy routes remain available at `/docs` and `/openapi.json`; do not assume legacy response paths, redirects, boolean format flags, or text errors apply to v2.

- Interactive documentation: `/api/v2/docs`
- OpenAPI document: `/api/v2/openapi.json`
- Base path: `/api/v2`

## Conventions

- Resource paths use canonical Twitch numeric ids, represented as strings.
- JSON fields use lower camel case.
- RFC 3339 timestamps are UTC-aware. A log range is `[from, to)`: `from` is inclusive and `to` is exclusive.
- A range requires both `from` and `to`; `to` must be later than `from`.
- Log and availability responses use `Cache-Control: no-cache` so an opt-out can withdraw data without relying on an expired shared cache.
- A channel or user blocked by opt-out policy receives `403` and is never returned in a bulk result.

## Errors

Application errors use `application/problem+json`:

```json
{
  "code": "invalid_request",
  "status": 400,
  "title": "The request is invalid"
}
```

Defined codes are `invalid_request`, `opted_out`, `not_found`, `upstream_unavailable`, and `internal_error`. Do not parse `title`; use `code` and HTTP status for program logic.

## Resolve a login

```text
GET /api/v2/users/resolve?login=example_channel
```

```json
{
  "id": "123456",
  "login": "example_channel"
}
```

The returned `id` can be used in the remaining v2 paths.

## List available periods

```text
GET /api/v2/channels/123456/availability
GET /api/v2/channels/123456/availability?userId=987654
```

Without `userId`, the response lists channel day buckets. With `userId`, it lists user month buckets:

```json
{
  "availableLogs": [
    { "year": "2026", "month": "03" }
  ]
}
```

## Read user logs

```text
GET /api/v2/channels/123456/users/987654/logs?from=2026-03-01T00:00:00Z&to=2026-04-01T00:00:00Z&format=basic-json
```

Supported `format` values:

| Value | Content type | Payload |
| --- | --- | --- |
| `basic-json` | `application/json` | `{ "messages": [BasicMessage] }` |
| `full-json` | `application/json` | `{ "messages": [FullMessage] }` |
| `ndjson` | `application/x-ndjson` | one basic message per line |
| `text` | `text/plain; charset=utf-8` | formatted message lines |
| `raw` | `text/plain; charset=utf-8` | raw IRC message lines |

`basic-json` is the default. Empty JSON responses are valid JSON with an empty `messages` array.

Optional query parameters:

- `reverse=true` reverses result order.
- `limit` is a positive integer.
- `offset` is a zero-based number of matching messages to skip.

Clients consuming live events should retain the latest message timestamp and id, replay this endpoint after reconnecting, and de-duplicate events. The admin firehose is at-least-once delivery, not a durable cursor.

## Admin live feed

`GET /admin/firehose` is intentionally outside the browser-oriented v2 surface. It requires the `X-Api-Key` header and supports `format=raw` or `format=json-basic`. Never place the API key in a query parameter, URL fragment, or WebSocket subprotocol.

See [CONFIG.md](./CONFIG.md) for authentication, slow-client close behavior, and reverse-proxy guidance.
