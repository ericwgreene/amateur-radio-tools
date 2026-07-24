# Deploying to Azure Container Apps

The app runs as a container on **Azure Container Apps**, with the image in **Azure
Container Registry (ACR)** and data in **Azure Database for PostgreSQL Flexible Server**.
Infrastructure is defined in [`infra/main.bicep`](../infra/main.bicep); CI/CD lives in
[`.github/workflows`](../.github/workflows).

## What gets provisioned

Container Registry · Log Analytics workspace · Container Apps environment · Container App
(external HTTPS ingress on `:8080`) · a migration **Container Apps Job** · PostgreSQL
Flexible Server (public + firewall, SSL required) · Key Vault (RBAC) with the app secrets ·
two user-assigned managed identities (app runtime + GitHub OIDC deploy) with role
assignments and federated credentials.

Secrets (`database-url`, `session-secret`, `auth0-client-secret`) live in Key Vault and are
referenced by the Container App via its managed identity — never stored in GitHub or baked
into the image.

## One-time setup

### 1. Provision infrastructure

Requires the Azure CLI and a principal that is **Owner** (or Contributor + User Access
Administrator) on the resource group **and** **Key Vault Secrets Officer** (Bicep writes
secret values into the RBAC vault).

```bash
az group create -n rg-amateur-radio-tools -l eastus

az deployment group create \
  -g rg-amateur-radio-tools \
  -f infra/main.bicep \
  -p namePrefix=art \
     postgresAdminLogin=artadmin \
     postgresAdminPassword="$(openssl rand -base64 24)" \
     sessionSecret="$(openssl rand -hex 64)" \
     clientAllowedIp="$(curl -s ifconfig.me)"   # optional: your IP for psql access
```

On the first apply the Container App starts on a **public placeholder image**
(`containerapps-helloworld`) because ACR is still empty — that revision will read unhealthy
on `:8080` until the first real deploy. (RBAC can take a minute to propagate; if the deploy
fails resolving a Key Vault secret, just re-run it.)

### 2. Set GitHub repository variables

Copy the deployment outputs into repo **Variables** (Settings → Secrets and variables →
Actions → Variables — these are non-secret):

| Repo variable | Bicep output |
|---|---|
| `AZURE_CLIENT_ID` | `githubIdentityClientId` |
| `AZURE_TENANT_ID` | `tenantId` |
| `AZURE_SUBSCRIPTION_ID` | `subscriptionId` |
| `ACR_NAME` | `acrName` |
| `RESOURCE_GROUP` | `resourceGroupName` |
| `CONTAINERAPP_NAME` | `containerAppName` |
| `MIGRATION_JOB_NAME` | `migrationJobName` |
| `CONTAINERAPP_FQDN` | `containerAppFqdn` |

```bash
az deployment group show -g rg-amateur-radio-tools -n main --query properties.outputs
```

Also create a GitHub **Environment** named `production` (Settings → Environments) — the
deploy job binds to it, matching the federated credential subject.

Until `AZURE_CLIENT_ID` is set, the deploy workflow is a **graceful no-op** (it doesn't fail).

### 3. First deploy

Trigger the deploy workflow (push to `main`, or run it manually via *workflow_dispatch*). It
builds the image with `az acr build`, runs the migration Job, then rolls the Container App to
the new image and smoke-tests `/health`.

### 4. Wire up Auth0 (optional but expected)

The app boots with **authentication disabled** until Auth0 is configured, so you can validate
infra first. To enable it:

1. In the Auth0 application, add the deployed FQDN:
   - Allowed Callback URLs: `https://<CONTAINERAPP_FQDN>/auth/callback`
   - Allowed Logout URLs: `https://<CONTAINERAPP_FQDN>/`
2. Re-run the Bicep deploy with the Auth0 params set:
   ```bash
   az deployment group create -g rg-amateur-radio-tools -f infra/main.bicep \
     -p namePrefix=art postgresAdminLogin=artadmin postgresAdminPassword=... \
        sessionSecret=... \
        auth0Domain=your-tenant.us.auth0.com \
        auth0ClientId=... auth0ClientSecret=...
   ```

`BASE_URL` is computed from the environment's default domain automatically, so the OAuth
callback is correct without any manual URL edits. (For a stable URL, bind a custom domain and
pre-register it in Auth0.)

## How the pipeline works

- **`ci.yml`** (pull requests + pushes to main): `cargo fmt --check`, `cargo clippy -D
  warnings`, and `cargo-llvm-cov` with **`--fail-under-lines 80`**. Runs fully offline against
  in-memory SQLite — no Azure, Postgres, or Node.
- **`deploy.yml`** (push to main + manual): OIDC login → `az acr build` (server-side, no local
  Docker) → migration Job → `az containerapp update --image` → `/health` smoke test. Images are
  tagged with the commit SHA (immutable) and `latest`.

Migrations run via the dedicated Job **before** the app is rolled, so the app's on-boot
`Migrator::up` finds nothing pending — avoiding a multi-replica migration race.

## Notes & future hardening

- **Secret rotation:** update the Key Vault secret, then create a new Container App revision
  (a deploy) to pick it up. The `database-url` secret embeds the DB password, so rotating the
  password means rewriting that secret.
- **PostgreSQL** uses password auth via Key Vault; Entra/managed-identity DB auth is a natural
  next hardening step.
- **Networking** is public + firewall; a VNet-integrated environment + private Postgres is the
  more locked-down alternative.
- The Bicep in this repo has **not** been compiled in CI yet; validate with
  `az bicep build -f infra/main.bicep` (or `az deployment group what-if`) before the first
  apply. Adding a `bicep build` lint step to CI is a good follow-up.
