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
| `method_not_allowed` | 405 | The endpoint does not support the HTTP method. |
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
Logins in stats and tiers are left out when Twitch does not know the user
or cannot be asked; the counts do not depend on Twitch.

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

Dates and months are UTC, newest first. Log ranges are required; user stats
take an optional range. Search looks through all of the user's messages in
the channel, ignoring case.

Every endpoint that returns messages takes `format`:

| `format` | Content type | Payload |
| --- | --- | --- |
| `basic-json` (default) | `application/json` | `{ "messages": [BasicMessage] }` |
| `full-json` | `application/json` | `{ "messages": [FullMessage] }` |
| `ndjson` | `application/x-ndjson` | one `BasicMessage` per line |
| `text` | `text/plain; charset=utf-8` | formatted lines |
| `raw` | `text/plain; charset=utf-8` | raw IRC lines |

Lists of messages also take `reverse=true` (newest first), `limit` (at least
1) and `offset` (messages to skip). A list without messages is empty in its
format (`{ "messages": [] }` for JSON), not an error; a random message of a
channel or user without messages is `404`.

To follow live chat, remember the timestamp and id of the latest message,
replay this endpoint after reconnecting, and drop the messages seen already:
the admin firehose delivers at least once and has no durable cursor.

### Tiers

```text
GET /channels/{channelId}/tiers/{period}?mode=all&excludeBots=nightbot&excludeBots=moobot
```

`period` is a calendar day (`2026-03-01`), month (`2026-03`) or year
(`2026`) in the Europe/Moscow time zone. `mode` counts all messages (`all`,
the default), only those sent while the stream was live (`online`) or while
it was offline (`offline`), as SullyGnome knows the streams. `excludeBots`,
repeated for several logins, replaces the default list of bots left out of
the table; `excludeBots=` alone leaves out none.

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
      "tierScore": 50,
      "windows": {
        "1m": { "active": 3, "rank": 1, "tier": "HT1" },
        "5m": { "active": 3, "rank": 1, "tier": "HT1" },
        "15m": { "active": 2, "rank": 1, "tier": "HT1" },
        "30m": { "active": 2, "rank": 1, "tier": "HT1" },
        "60m": { "active": 1 }
      }
    }
  ]
}
```

`windows` has the numbers of active 1, 5, 15, 30 and 60 minute windows, with
the rank and tier within each size where the user is ranked. Tiers go from
`HT1` (high tier 1, the best) through `LT1`, `HT2` and so on to `LT5`. Up to
500 entries are returned.

### Opt-out

Users and channels opt out of logging, and back in, with a chat command. It
needs no account on this side: Twitch vouches for who sent the message.

1. `POST /opt-out-codes` answers `201` with `{ "code", "expiresAt" }`: a
   one-time code, valid for a minute.
2. The command with the code is written in a Twitch chat:

| Command | Where | Effect |
| --- | --- | --- |
| `!rustlog optout <code>` | any logged chat | Stops logging the sender and deletes their messages and logins in every channel. |
| `!rustlog optin <code>` | any logged chat | Logs the sender again from now on; deleted messages stay deleted. |
| `!rustlog optout-channel <code>` | the channel's own chat, by its broadcaster | Stops logging the channel and hides its logs, which are kept. |
| `!rustlog optin-channel <code>` | the channel's own chat, by its broadcaster | Logs the channel again and shows its logs, old ones included. |

The bot stays in the chat of a channel that opted out, so that its
broadcaster can opt back in there; the channel is left out of
`GET /channels`. Admins (the `admins` of the config) can write any of the
commands with a login instead of a code to act for that user or channel,
and the admin API below does the same.

### Admin

Admin requests need the `X-Api-Key` header; never put the key in a URL. A
missing or wrong key is answered with `401`.

| Request | Effect (`204`) |
| --- | --- |
| `PUT /admin/channels/{channelId}` | Start logging the channel. |
| `DELETE /admin/channels/{channelId}` | Stop logging the channel; its logs stay visible. |
| `PUT /admin/channels/{channelId}/opt-out` | Opt the channel out: stop logging it and hide its logs. |
| `DELETE /admin/channels/{channelId}/opt-out` | Opt the channel back in. |
| `PUT /admin/users/{userId}/opt-out` | Opt the user out: stop logging them and delete their messages. |
| `DELETE /admin/users/{userId}/opt-out` | Opt the user back in. |

The live WebSocket feed stays at `GET /admin/firehose`, outside v2. See
[CONFIG.md](./CONFIG.md) for its formats and slow-client behavior.
