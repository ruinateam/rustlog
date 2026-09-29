Chat logs of the logged Twitch channels, message stats, chat tiers and
stream history.

## Conventions

- Channels and users are addressed by their numeric Twitch ids. Look up the
  id of a login with `GET /users?login=...`; logins change, ids do not.
- JSON fields are in lower camel case; absent optional fields are left out.
- Timestamps are RFC 3339 in UTC, dates `YYYY-MM-DD`, months `YYYY-MM`. A
  time range `[from, to)` includes `from` and excludes `to`.
- A list parameter repeats: `?login=a&login=b`.
- Data of channels and users that opted out is answered with `403`.

## Errors

Errors are `application/problem+json` (RFC 9457). Use `code` and the HTTP
status in program logic; `title` and `detail` are for people.

| `code` | Status |
| --- | --- |
| `invalid_request` | 400 |
| `unauthorized` | 401 |
| `opted_out` | 403 |
| `not_found` | 404 |
| `method_not_allowed` | 405 |
| `internal_error` | 500 |
| `upstream_unavailable` | 503 |
