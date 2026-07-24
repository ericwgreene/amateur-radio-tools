# Amateur Radio Tools

A server-rendered web application scaffold for a suite of amateur-radio utilities.

- **[Rust](https://www.rust-lang.org/) + [Actix Web](https://actix.rs/)** — the HTTP server.
- **[Askama](https://github.com/rinja-rs/askama)** — compile-time, type-safe HTML templates
  (integrated with Actix via [`askama_web`](https://crates.io/crates/askama_web)).
- **[HTMX](https://htmx.org/)** — small, targeted client-side interactivity over
  server-rendered HTML fragments. The app is server-rendered first; HTMX enhances it.
- **[SeaORM](https://www.sea-ql.org/SeaORM/)** — async ORM over **SQLite** by default, with a
  clean path to **PostgreSQL**.
- **[Auth0](https://auth0.com/)** — authentication & authorization via OpenID Connect
  (Authorization Code flow with PKCE).
- **[Tailwind CSS](https://tailwindcss.com/)** — styling, built with the standalone CLI.

The app boots and runs **without any configuration** (authentication simply stays
disabled), so you can start immediately and wire up Auth0 when you're ready.

---

## What's included

### Browser (server-rendered + HTMX)

| Route | Auth | Description |
|-------|------|-------------|
| `GET /` | public | Home page. Hosts the **Maidenhead grid-square** and **callsign lookup** tools (HTMX partial-update demos). |
| `POST /tools/grid` | public | HTMX: converts lat/long → Maidenhead locator. |
| `POST /tools/callsign` | public | HTMX: resolves a callsign's country/continent. |
| `GET /dashboard` | **required** | The signed-in user's profile & account timestamps. |
| `GET /logbook` | **required** | The **logbook**: list contacts, add one (with live callsign lookup), delete — all via HTMX. |
| `POST /logbook`, `DELETE /logbook/{id}` | **required** | Add / remove a contact; return the refreshed table body. |
| `GET /stations` | **required** | **Stations**: every callsign ever heard, once each, with live search and sorting. |
| `GET /stations/{callsign}` | **required** | One station: its details and every time it was heard. |
| `GET /sessions` · `GET /sessions/{id}` | **required** | Monitoring runs, nets, and contests, and what each turned up. |
| `POST /observations/{id}/promote` | **required** | HTMX: copy a heard station into the logbook as a QSO. |
| `GET /settings/tokens` | **required** | Create & revoke **API tokens**. |
| `POST /settings/tokens`, `POST /settings/tokens/{id}/revoke` | **required** | Manage tokens (the secret is shown once). |
| `GET /admin` | **role: `admin`** | Lists all users — demonstrates role-based authorization. |
| `GET /login` · `GET /auth/callback` · `GET /logout` | — | Auth0 OIDC login / callback / logout. |
| `GET /health` | — | Liveness probe. |

### REST API (JSON, token-authenticated)

| Route | Description |
|-------|-------------|
| `POST /api/v1/contacts` | Log a contact. |
| `GET /api/v1/contacts` | List the caller's contacts. |
| `GET /api/v1/contacts/{id}` | Fetch one of the caller's contacts. |
| `GET /api/v1/me` | Identify the token's owner (handy for verifying a token). |
| `POST /api/v1/sessions` | Open a monitoring/contest session (idempotent). |
| `PATCH /api/v1/sessions/by-key/{client_key}` | Relabel or close a session. |
| `GET /api/v1/sessions` · `GET /api/v1/sessions/{id}` | List / fetch sessions. |
| `POST /api/v1/observations` | **Batch**-log heard transmissions. |
| `GET /api/v1/observations` | The monitoring log, filterable and paged. |
| `POST /api/v1/observations/{id}/promote` | Turn a heard station into a logbook contact. |
| `GET /api/v1/stations` · `GET /api/v1/stations/{callsign}` | The unique-station roster. |

See [REST API](#rest-api) below.

### A note on the login form

There is **no custom username/password form** — and there shouldn't be. Auth0 hosts the
login experience (Universal Login), so `/login` simply redirects to Auth0, which handles
credentials, social logins, and MFA. Building an in-app credential form would mean using the
discouraged Resource Owner Password grant and taking on password handling yourself. The
"Log in" button in the nav is all that's needed.

---

## Project layout

A Cargo workspace with three crates (SeaORM's recommended separation keeps the DB backend
swappable and the entities regenerable):

```
amateur-radio-tools/
├── Cargo.toml                 # workspace + centralized dependency versions
├── package.json               # Tailwind CSS build scripts
├── .env.example               # documented configuration
└── crates/
    ├── entity/                # SeaORM entities: users, contacts, api_tokens
    ├── migration/             # backend-agnostic migrations + a CLI runner
    └── web/                   # the Actix Web application
        ├── src/
        │   ├── main.rs        # bootstrap: config, DB+migrate, Auth0 discovery, server
        │   ├── config.rs      # environment configuration
        │   ├── state.rs       # shared AppState (DB, config, Auth0 client)
        │   ├── error.rs       # AppError + HTML error rendering
        │   ├── auth/          # OIDC client, session, role checks, API tokens
        │   ├── api/           # JSON REST API (/api/v1/*) + JSON error type
        │   ├── routes/        # pages, HTMX partials, logbook, settings
        │   └── tools/         # domain logic (Maidenhead, callsign), unit-tested
        ├── examples/seed.rs   # dev-only: mint a user + API token for local API testing
        ├── templates/         # Askama templates (layout, pages, partials)
        ├── assets/            # Tailwind input CSS
        └── static/            # built app.css + vendored htmx.min.js (served at /static)
```

---

## Prerequisites

- **Rust** (stable; see `rust-toolchain.toml`) — <https://rustup.rs>
- A C toolchain + **CMake** (transitively needed to build the TLS backend `aws-lc-rs`).
- **Node.js** — only to rebuild the Tailwind CSS. The pre-built `static/css/app.css` is
  committed, so you can run the app without Node.

---

## Quick start

```bash
# 1. (optional) configure — the app runs without this, with auth disabled
cp .env.example .env

# 2. run — connects to SQLite, applies migrations, and serves on http://localhost:8080
cargo run -p web
```

Open <http://localhost:8080>. The home page and the Maidenhead tool work immediately.
`/dashboard` redirects to `/login`; until Auth0 is configured, `/login` reports that
authentication is unavailable.

> Run from the workspace root so the default `STATIC_DIR=crates/web/static` and the SQLite
> path resolve correctly.

---

## Rebuilding the CSS

Tailwind scans the templates for class names and emits `crates/web/static/css/app.css`.

```bash
npm install          # once
npm run build:css    # one-off build (minified)
npm run watch:css    # rebuild on change during development
```

---

## Configuring Auth0

1. In the Auth0 Dashboard, create an application of type **Regular Web Application**.
2. In its settings, add these URLs (adjust the origin to match `BASE_URL`):
   - **Allowed Callback URLs:** `http://localhost:8080/auth/callback`
   - **Allowed Logout URLs:** `http://localhost:8080`
3. Copy the **Domain**, **Client ID**, and **Client Secret** into your `.env`:

   ```dotenv
   AUTH0_DOMAIN=your-tenant.us.auth0.com
   AUTH0_CLIENT_ID=...
   AUTH0_CLIENT_SECRET=...
   SESSION_SECRET=<output of: openssl rand -hex 64>
   ```
4. Restart. The startup log should read `Auth0 OIDC configured for tenant ...`.

The flow implemented is the OIDC **Authorization Code flow with PKCE**, with CSRF `state`
and replay-protecting `nonce` validation. On successful login the user is **upserted** into
the local `users` table (keyed by the Auth0 `sub`) and a minimal profile is stored in an
encrypted session cookie.

### Authorization (roles)

Auth0 delivers roles through a **namespaced custom claim** on the ID token, populated by a
post-login [Action](https://auth0.com/docs/customize/actions). Create one with this code:

```js
exports.onExecutePostLogin = async (event, api) => {
  const namespace = "https://amateur-radio-tools";
  const roles = event.authorization?.roles ?? [];
  api.idToken.setCustomClaim(`${namespace}/roles`, roles);
};
```

The claim name must match `AUTH0_ROLES_CLAIM` (default
`https://amateur-radio-tools/roles`). Assign an `admin` role to a user (Auth0 → User
Management → Roles) and they'll be able to reach `/admin`. Roles are read only *after* the
ID token has been cryptographically verified.

---

## Database & migrations

Migrations run **automatically on startup**. You can also drive them manually:

```bash
cargo run -p migration -- up        # apply pending migrations
cargo run -p migration -- down      # revert the last migration
cargo run -p migration -- status    # show status
cargo run -p migration -- fresh     # drop everything and re-apply
```

### Swapping to PostgreSQL

The migrations and queries are backend-agnostic, so switching is a two-step change:

1. Enable the Postgres driver features in `Cargo.toml` (`[workspace.dependencies]`):
   add `"sqlx-postgres"` to both `sea-orm` and `sea-orm-migration`.
2. Point `DATABASE_URL` at Postgres:
   `postgres://user:password@localhost:5432/amateur_radio_tools`

No application code changes are required.

---

## REST API

The API lets external applications (logging software, scripts, a station computer) record
contacts without the browser. It is authenticated with **personal API tokens** rather than
the browser session, because a logging program acts *as a specific user* and typically can't
run an interactive OAuth flow. (Auth0 remains the identity provider for humans in the
browser; tokens are the machine-to-machine complement.)

**Create a token:** sign in, go to **API tokens** (`/settings/tokens`), and create one. The
secret is shown once — copy it immediately.

**Use it:** send it as a bearer token.

```bash
# Verify a token
curl -H "Authorization: Bearer <token>" http://localhost:8080/api/v1/me

# Log a contact (only `callsign` is required; the country is auto-resolved)
curl -X POST http://localhost:8080/api/v1/contacts \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"callsign":"W1AW","band":"20m","mode":"SSB","frequency_mhz":14.074,"rst_sent":"59","rst_received":"59"}'

# List your contacts
curl -H "Authorization: Bearer <token>" http://localhost:8080/api/v1/contacts
```

**Header vs. query string.** The `Authorization: Bearer` header is the preferred method.
For clients that can't set custom headers, the token may instead be passed as the `api_key`
query parameter; if both are present, the header wins.

```bash
curl "http://localhost:8080/api/v1/me?api_key=<token>"
```

> Prefer the header where you can: credentials in the URL are more exposed than in a header
> — they end up in server/proxy access logs and browser history.

Tokens are stored only as a SHA-256 hash; missing/invalid tokens get a `401` with a JSON
body (`{"error":"unauthorized", ...}`), and malformed request bodies get a `400`.

### Monitoring: sessions, observations, stations

The contacts endpoints log **QSOs** — stations you worked. A receive-only monitor produces
something different: a stream of stations *heard*. Those are kept in their own tables so the
logbook stays a logbook (and a future ADIF export stays meaningful) while a busy net's
hundreds of hearings go somewhere they can't drown it.

Three resources:

- **`sessions`** — one operating run: a casual monitor, a net, a contest, a POTA activation.
  Carries the band/mode/frequency a receive-only setup can't discover for itself.
- **`observations`** — one heard transmission, belonging to a session.
- **`stations`** — the rollup: one row per callsign you have *ever* heard, with first/last
  heard and a hearing count. This is the "unique contacts over time" log.

In the browser these are **Stations** and **Sessions**. Heard a station and then actually
worked it? *Log as QSO* on the station page copies that hearing into the logbook — which
is the only thing that moves a row from one side to the other, since `times_worked` is
counted from `contacts` rather than stored.

```bash
# Upload a batch of hearings. The session travels with the batch and is created or
# updated as a side effect, so a client never has to learn a server-assigned id.
curl -X POST http://localhost:8080/api/v1/observations \
  -H "Authorization: Bearer <token>" -H "Content-Type: application/json" \
  -d '{
        "session": {"client_key":"run-2026-07-24","kind":"net","label":"Tuesday ARES net",
                    "band":"2m","mode":"FM","frequency_mhz":146.88},
        "observations": [
          {"client_key":"clip-1:KR4NRC","callsign":"KR4NRC","heard_at":"2026-07-24T23:12:41Z",
           "duration_secs":4.8,"name":"John Smith","qth":"Lynchburg, VA"}
        ]
      }'

# The roster of every station ever heard
curl -H "Authorization: Bearer <token>" "http://localhost:8080/api/v1/stations?order=times_heard"

# Close the session when the run ends
curl -X PATCH http://localhost:8080/api/v1/sessions/by-key/run-2026-07-24 \
  -H "Authorization: Bearer <token>" -H "Content-Type: application/json" \
  -d '{"ended_at":"2026-07-25T01:30:00Z"}'
```

Three behaviours are worth knowing, because they are what let an offline client retry
safely:

- **Ingest is idempotent.** Every item carries a `client_key`; replaying a batch after a
  network failure reports the rows as `duplicates` and inserts nothing.
- **A bad row never fails the batch.** The response is always `200` with
  `{accepted, duplicates, rejected, stations_touched}`. If one unparseable callsign returned
  `400` for the whole request, a retrying client would wedge forever.
- **`PATCH .../by-key/{key}` creates an unknown key** rather than 404ing, so a queued
  "session ended" that arrives before its "session opened" still lands correctly.

Batches are capped at **200 observations**. These endpoints are paged — `?limit=&offset=`,
default 100, max 500 — and return `{items, total, limit, offset}`, unlike the older
contacts endpoints which return a bare array.

> **Note:** there is currently no per-token rate limiting, and tokens have no scopes or
> expiry. Batch ingest makes a misbehaving client more costly than before; the 200-item cap
> and the 2 MB request-body limit are the only ceilings today.

### Testing the API without Auth0

Since a token is tied to a user, you normally need to log in first. For local development
without an Auth0 tenant, a seeding example mints a user + token directly:

```bash
cargo run -p web --example seed          # prints: API_TOKEN=art_...
```

Then use the printed token against a locally running server.

---

## Testing, formatting, linting

```bash
cargo test --workspace     # domain logic: Maidenhead, callsign, token hashing, datetime
cargo fmt --all
cargo clippy --workspace --all-targets
```

---

## Configuration reference

See `.env.example` for the full list. Highlights:

| Variable | Default | Notes |
|----------|---------|-------|
| `BIND_ADDRESS` | `127.0.0.1:8080` | Socket to bind. |
| `BASE_URL` | `http://localhost:8080` | Must match Auth0's registered URLs. |
| `DATABASE_URL` | `sqlite://./data/…?mode=rwc` | SQLite or Postgres URL. |
| `SESSION_SECRET` | *(ephemeral)* | 64+ bytes; `openssl rand -hex 64`. |
| `COOKIE_SECURE` | `false` | **Set `true` in production (HTTPS).** |
| `AUTH0_DOMAIN` / `AUTH0_CLIENT_ID` / `AUTH0_CLIENT_SECRET` | — | Blank ⇒ auth disabled. |
| `AUTH0_ROLES_CLAIM` | `https://amateur-radio-tools/roles` | Namespaced roles claim. |
| `STATIC_DIR` | `crates/web/static` | Directory served at `/static`. |
| `RUST_LOG` | `info,web=debug` | Standard `tracing`/`env_logger` filter. |

---

## Production notes

- Set a persistent `SESSION_SECRET` and `COOKIE_SECURE=true` (serve behind HTTPS/TLS).
- Build in release mode: `cargo build -p web --release` (binary: `amateur-radio-tools`).
- Ship the built `static/` directory and point `STATIC_DIR` at it.
- Consider running the app behind a reverse proxy that terminates TLS.

## License

Licensed under either of MIT or Apache-2.0 at your option.
