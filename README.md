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

## What's included (the walking skeleton)

| Route | Auth | Description |
|-------|------|-------------|
| `GET /` | public | Home page. Hosts the **Maidenhead grid-square** tool (an HTMX partial-update demo). |
| `POST /tools/grid` | public | HTMX endpoint: converts lat/long → Maidenhead locator, returns an HTML fragment. |
| `GET /dashboard` | **required** | The signed-in user's profile & account timestamps (reads the local DB). |
| `GET /admin` | **role: `admin`** | Lists all users — demonstrates role-based authorization. |
| `GET /login` | — | Starts the Auth0 login flow. |
| `GET /auth/callback` | — | OIDC redirect URI: verifies the ID token, upserts the user, sets the session. |
| `GET /logout` | — | Clears the session and ends the Auth0 session. |
| `GET /health` | — | Liveness probe. |

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
    ├── entity/                # SeaORM entities (the `users` table)
    ├── migration/             # backend-agnostic migrations + a CLI runner
    └── web/                   # the Actix Web application
        ├── src/
        │   ├── main.rs        # bootstrap: config, DB+migrate, Auth0 discovery, server
        │   ├── config.rs      # environment configuration
        │   ├── state.rs       # shared AppState (DB, config, Auth0 client)
        │   ├── error.rs       # AppError + HTML error rendering
        │   ├── auth/          # Auth0 OIDC client, session, login/callback/logout
        │   ├── routes/        # page + HTMX-partial handlers
        │   └── tools/         # domain logic (Maidenhead), unit-tested, web-free
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

## Testing, formatting, linting

```bash
cargo test --workspace     # includes the Maidenhead locator unit tests
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
