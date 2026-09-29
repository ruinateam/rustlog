# API v2

API v2 covers everything the legacy API does, with consistent conventions. The
legacy API stays frozen for existing clients and is deprecated.

- Interactive documentation of both APIs: `/docs`
- OpenAPI documents: `/api/v2/openapi.json` (v2), `/openapi.json` (legacy)
- Base path: `/api/v2`

## Conventions

- **Ids.** Channels and users are addressed by their numeric Twitch ids,
  represented as strings. Look up the id of a login with `GET /users`. Twitch
  logins change; ids do not.
- **JSON.** Fields are in lower camel case. Absent optional fields are left
  out rather than `null`.
- **Time.** Timestamps are RFC 3339 in UTC. Dates are `YYYY-MM-DD`, months
  `YYYY-MM`. A time range is half-open, `[from, to)`: `from` is inclusive,
  `to` exclusive, and `to` must be later than `from`.
- **Opt-out.** Data of a channel or user that opted out is answered with
  `403 opted_out` and never appears in lists. Log, stats and availability
  responses carry `Cache-Control: no-cache`, so an opt-out takes effect
  without waiting for a shared cache to expire.

## Errors

Every error is an `application/problem+json` body in the shape of
[RFC 9457](https://www.rfc-editor.org/rfc/rfc9457):

```json
{
  "code": "invalid_request",
  "status": 400,
  "title": "The request is invalid",
  "detail": "`to` must be later than `from`"
}
```

Use `code` and the HTTP status in program logic; `title` and `detail` are for
people and may change. `detail` is left out when there is nothing to add.

| `code` | Status | Meaning |
| --- | --- | --- |
| `invalid_request` | 400 | A parameter is missing or malformed. |
| `unauthorized` | 401 | The admin API key is missing or wrong. |
| `opted_out` | 403 | The channel or user opted out of logging. |
| `not_found` | 404 | No such route, channel, user or data. |
| `upstream_unavailable` | 503 | Twitch cannot be queried yet. |
| `internal_error` | 500 | Something went wrong on the server. |

## Endpoints

### Users

| Request | Response |
| --- | --- |
| `GET /users?login=a&login=b` | `{ "users": [User] }` |
| `GET /users?id=1&id=2` | `{ "users": [User] }` |
| `GET /users/{userId}/name-history` | `{ "names": [PreviousName] }` |

`GET /users` takes up to 100 logins or ids in total, repeating the parameter.
Users Twitch does not know are left out. A `User` is `{ "id", "login" }`; a
`PreviousName` is `{ "login", "firstSeenAt", "lastSeenAt" }`.

### Channels

| Request | Response |
| --- | --- |
| `GET /channels` | `{ "channels": [User] }`, the logged channels |
| `GET /channels/{channelId}/badges` | `{ "badges": [ChatBadge] }` |
| `GET /channels/{channelId}/stats` | `ChannelStats` |
| `GET /channels/{channelId}/streams?year=2026` | `{ "streams": [Stream] }` |

Badges are the global and the channel's Twitch chat badges, for logged
channels only: `{ "setId", "version", "title", "description", "imageUrl1x",
"imageUrl2x" }`.

Stats take an optional range (`from` and `to`) and return
`{ "messageCount", "topChatters": [{ "userId", "login", "messageCount" }] }`.

Streams come from SullyGnome: `{ "id", "startedAt", "endedAt",
"durationMinutes", "games" }`, where unknown fields are left out.

### Logs

| Request | Response |
| --- | --- |
| `GET /channels/{channelId}/log-dates` | `{ "dates": ["2026-03-01", ...] }` |
| `GET /channels/{channelId}/logs?from&to` | messages |
| `GET /channels/{channelId}/logs/random` | one message |
| `GET /channels/{channelId}/users/{userId}/log-months` | `{ "months": ["2026-03", ...] }` |
| `GET /channels/{channelId}/users/{userId}/logs?from&to` | messages |
| `GET /channels/{channelId}/users/{userId}/logs/random` | one message |
| `GET /channels/{channelId}/users/{userId}/logs/search?q=` | messages |
| `GET /channels/{channelId}/users/{userId}/stats` | `{ "userId", "login", "messageCount" }` |

Dates and months are UTC, newest first. Log ranges are required; search and
user stats take an optional range.

Every endpoint that returns messages takes `format`:

| `format` | Content type | Payload |
| --- | --- | --- |
| `basic-json` (default) | `application/json` | `{ "messages": [BasicMessage] }` |
| `full-json` | `application/json` | `{ "messages": [FullMessage] }` |
| `ndjson` | `application/x-ndjson` | one `BasicMessage` per line |
| `text` | `text/plain; charset=utf-8` | formatted lines |
| `raw` | `text/plain; charset=utf-8` | raw IRC lines |

Lists of messages also take `reverse=true` (newest first), `limit` (at least
1) and `offset` (messages to skip).

To follow live chat, remember the timestamp and id of the latest message,
replay this endpoint after reconnecting, and drop the messages seen already:
the admin firehose delivers at least once and has no durable cursor.

### Tiers

```text
GET /channels/{channelId}/tiers/{period}?mode=all&excludeBots=nightbot,moobot
```

`period` is a calendar day (`2026-03-01`), month (`2026-03`) or year
(`2026`) in the Europe/Moscow time zone. `mode` counts all messages (`all`,
the default), only those sent while the stream was live (`online`) or while
it was offline (`offline`). `excludeBots` replaces the default list of bots
left out of the table.

```json
{
  "period": "2026-03",
  "timezone": "Europe/Moscow",
  "mode": "all",
  "totalUsers": 2,
  "totalMessages": 5,
  "totalUniqueMessages": 5,
  "entries": [
    {
      "userId": "22222",
      "login": "alice",
      "messages": 3,
      "uniqueMessages": 3,
      "tierScore": 12,
      "windows": {
        "1m": { "active": 3, "rank": 1, "tier": "S" },
        "5m": { "active": 3 }
      }
    }
  ]
}
```

`windows` has the numbers of active 1, 5, 15, 30 and 60 minute windows, with
the rank and tier within each size where the user is ranked. Up to 500
entries are returned.

### Opt-out

`POST /opt-out-codes` answers `201` with `{ "code", "expiresAt" }`. Writing
`!rustlog optout <code>` in the chat of a logged channel before the code
expires, a minute later, opts the sender out.

### Admin

Admin requests need the `X-Api-Key` header; never put the key in a URL.

| Request | Effect |
| --- | --- |
| `PUT /admin/channels/{channelId}` | Start logging the channel (`204`). |
| `DELETE /admin/channels/{channelId}` | Stop logging the channel (`204`). |

The live WebSocket feed stays at `GET /admin/firehose`, outside v2. See
[CONFIG.md](./CONFIG.md) for its formats and slow-client behavior.
