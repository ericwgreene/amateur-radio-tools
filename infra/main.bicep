// Azure infrastructure for amateur-radio-tools.
//
// Provisions everything needed to run the app on Azure Container Apps:
//   ACR · Log Analytics · Container Apps environment · Container App · migration Job ·
//   PostgreSQL Flexible Server · Key Vault (RBAC) · two managed identities (app runtime +
//   GitHub OIDC deploy) · federated credentials · role assignments.
//
// Deploy (once) with a bootstrap principal that is Owner (or Contributor + User Access
// Administrator) on the resource group AND Key Vault Secrets Officer:
//
//   az group create -n rg-art -l eastus
//   az deployment group create -g rg-art -f infra/main.bicep \
//       -p namePrefix=art postgresAdminLogin=artadmin \
//          postgresAdminPassword=... sessionSecret=$(openssl rand -hex 64)
//
// The GitHub deploy workflow then builds/pushes the image and rolls the Container App.

targetScope = 'resourceGroup'

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

@description('Azure region for all resources.')
param location string = resourceGroup().location

@description('Short prefix used to derive resource names, e.g. "art".')
@minLength(2)
@maxLength(12)
param namePrefix string

@description('PostgreSQL administrator login (plain username, no @server suffix).')
param postgresAdminLogin string

@secure()
@description('PostgreSQL administrator password.')
param postgresAdminPassword string

@secure()
@description('Session cookie secret; must be at least 64 bytes (e.g. `openssl rand -hex 64`).')
param sessionSecret string

@description('Auth0 tenant domain (no scheme). Leave empty to deploy with auth disabled.')
param auth0Domain string = ''

@description('Auth0 application client id. Leave empty to deploy with auth disabled.')
param auth0ClientId string = ''

@secure()
@description('Auth0 application client secret. Leave empty to deploy with auth disabled.')
param auth0ClientSecret string = ''

@description('Container image the app runs. Defaults to a public placeholder for the first deploy; CI updates it to the ACR image afterwards.')
param containerImage string = 'mcr.microsoft.com/azuredocs/containerapps-helloworld:latest'

@description('Optional client/CI IPv4 address to allow through the PostgreSQL firewall. Empty skips the rule.')
param clientAllowedIp string = ''

@description('GitHub repository in owner/repo form, used for OIDC federated credential subjects.')
param githubRepo string = 'ericwgreene/amateur-radio-tools'

@description('GitHub Actions environment name the deploy job binds to.')
param githubEnvironment string = 'production'

// ---------------------------------------------------------------------------
// Names & role definition ids
// ---------------------------------------------------------------------------

var suffix = uniqueString(resourceGroup().id)
var acrName = toLower('${namePrefix}acr${suffix}')
var lawName = '${namePrefix}-law'
var envName = '${namePrefix}-env'
var appName = '${namePrefix}-web'
var jobName = '${namePrefix}-migrate'
// Key Vault names are capped at 24 chars, so use a shortened unique suffix.
var kvName = toLower('${namePrefix}kv${substring(suffix, 0, 6)}')
var pgName = toLower('${namePrefix}-pg-${suffix}')
var dbName = 'amateur_radio_tools'
var uamiAppName = '${namePrefix}-app-id'
var uamiGithubName = '${namePrefix}-github-id'

var roleAcrPull = subscriptionResourceId('Microsoft.Authorization/roleDefinitions', '7f951dda-4ed3-4680-a7ca-43fe172d538d')
var roleAcrPush = subscriptionResourceId('Microsoft.Authorization/roleDefinitions', '8311e382-0749-4cb8-b61a-304f252e45ec')
var roleKvSecretsUser = subscriptionResourceId('Microsoft.Authorization/roleDefinitions', '4633458b-17de-408a-b874-0445c86b69e6')
var roleContributor = subscriptionResourceId('Microsoft.Authorization/roleDefinitions', 'b24988ac-6180-42a0-ab88-20f7382dd24c')

var auth0Enabled = !empty(auth0Domain) && !empty(auth0ClientId) && !empty(auth0ClientSecret)

// ---------------------------------------------------------------------------
// Container Registry
// ---------------------------------------------------------------------------

resource acr 'Microsoft.ContainerRegistry/registries@2023-07-01' = {
  name: acrName
  location: location
  sku: { name: 'Basic' }
  properties: {
    adminUserEnabled: false
  }
}

// ---------------------------------------------------------------------------
// Log Analytics + Container Apps environment
// ---------------------------------------------------------------------------

resource law 'Microsoft.OperationalInsights/workspaces@2023-09-01' = {
  name: lawName
  location: location
  properties: {
    sku: { name: 'PerGB2018' }
    retentionInDays: 30
  }
}

resource managedEnv 'Microsoft.App/managedEnvironments@2024-03-01' = {
  name: envName
  location: location
  properties: {
    appLogsConfiguration: {
      destination: 'log-analytics'
      logAnalyticsConfiguration: {
        customerId: law.properties.customerId
        sharedKey: law.listKeys().primarySharedKey
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Managed identities + GitHub OIDC federation
// ---------------------------------------------------------------------------

resource uamiApp 'Microsoft.ManagedIdentity/userAssignedIdentities@2023-01-31' = {
  name: uamiAppName
  location: location
}

resource uamiGithub 'Microsoft.ManagedIdentity/userAssignedIdentities@2023-01-31' = {
  name: uamiGithubName
  location: location
}

// Deploy job bound to a GitHub Environment (recommended).
resource fedEnvironment 'Microsoft.ManagedIdentity/userAssignedIdentities/federatedIdentityCredentials@2023-01-31' = {
  parent: uamiGithub
  name: 'github-environment'
  properties: {
    issuer: 'https://token.actions.githubusercontent.com'
    subject: 'repo:${githubRepo}:environment:${githubEnvironment}'
    audiences: ['api://AzureADTokenExchange']
  }
}

// Fallback for jobs that run on a direct push to main without an environment binding.
resource fedMain 'Microsoft.ManagedIdentity/userAssignedIdentities/federatedIdentityCredentials@2023-01-31' = {
  parent: uamiGithub
  name: 'github-main'
  properties: {
    issuer: 'https://token.actions.githubusercontent.com'
    subject: 'repo:${githubRepo}:ref:refs/heads/main'
    audiences: ['api://AzureADTokenExchange']
  }
}

// ---------------------------------------------------------------------------
// Key Vault + secrets
// ---------------------------------------------------------------------------

resource keyVault 'Microsoft.KeyVault/vaults@2023-07-01' = {
  name: kvName
  location: location
  properties: {
    tenantId: subscription().tenantId
    sku: { family: 'A', name: 'standard' }
    enableRbacAuthorization: true
    enableSoftDelete: true
    publicNetworkAccess: 'Enabled'
  }
}

// The full connection string is stored as ONE secret: Container Apps maps a secretRef to
// the entire env value (no interpolation), so the URL cannot be assembled from fragments.
resource secretDatabaseUrl 'Microsoft.KeyVault/vaults/secrets@2023-07-01' = {
  parent: keyVault
  name: 'database-url'
  properties: {
    value: 'postgres://${postgresAdminLogin}:${uriComponent(postgresAdminPassword)}@${postgres.properties.fullyQualifiedDomainName}:5432/${dbName}?sslmode=require'
  }
}

resource secretSessionSecret 'Microsoft.KeyVault/vaults/secrets@2023-07-01' = {
  parent: keyVault
  name: 'session-secret'
  properties: {
    value: sessionSecret
  }
}

resource secretAuth0 'Microsoft.KeyVault/vaults/secrets@2023-07-01' = if (auth0Enabled) {
  parent: keyVault
  name: 'auth0-client-secret'
  properties: {
    value: auth0ClientSecret
  }
}

// ---------------------------------------------------------------------------
// PostgreSQL Flexible Server (public + firewall, SSL required)
// ---------------------------------------------------------------------------

resource postgres 'Microsoft.DBforPostgreSQL/flexibleServers@2024-08-01' = {
  name: pgName
  location: location
  sku: { name: 'Standard_B1ms', tier: 'Burstable' }
  properties: {
    version: '16'
    administratorLogin: postgresAdminLogin
    administratorLoginPassword: postgresAdminPassword
    storage: { storageSizeGB: 32 }
    highAvailability: { mode: 'Disabled' }
    authConfig: { passwordAuth: 'Enabled', activeDirectoryAuth: 'Disabled' }
    network: { publicNetworkAccess: 'Enabled' }
  }
}

// Allow the Container App's (dynamic) Azure egress to reach the server.
resource pgAllowAzure 'Microsoft.DBforPostgreSQL/flexibleServers/firewallRules@2024-08-01' = {
  parent: postgres
  name: 'AllowAllAzureServices'
  properties: {
    startIpAddress: '0.0.0.0'
    endIpAddress: '0.0.0.0'
  }
}

resource pgAllowClient 'Microsoft.DBforPostgreSQL/flexibleServers/firewallRules@2024-08-01' = if (!empty(clientAllowedIp)) {
  parent: postgres
  name: 'AllowClientIp'
  properties: {
    startIpAddress: clientAllowedIp
    endIpAddress: clientAllowedIp
  }
}

resource pgDatabase 'Microsoft.DBforPostgreSQL/flexibleServers/databases@2024-08-01' = {
  parent: postgres
  name: dbName
  properties: {
    charset: 'UTF8'
    collation: 'en_US.utf8'
  }
}

// ---------------------------------------------------------------------------
// Role assignments
// ---------------------------------------------------------------------------

resource acrPullApp 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(acr.id, uamiApp.id, roleAcrPull)
  scope: acr
  properties: {
    principalId: uamiApp.properties.principalId
    roleDefinitionId: roleAcrPull
    principalType: 'ServicePrincipal'
  }
}

resource kvSecretsUserApp 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(keyVault.id, uamiApp.id, roleKvSecretsUser)
  scope: keyVault
  properties: {
    principalId: uamiApp.properties.principalId
    roleDefinitionId: roleKvSecretsUser
    principalType: 'ServicePrincipal'
  }
}

resource acrPushGithub 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(acr.id, uamiGithub.id, roleAcrPush)
  scope: acr
  properties: {
    principalId: uamiGithub.properties.principalId
    roleDefinitionId: roleAcrPush
    principalType: 'ServicePrincipal'
  }
}

resource contributorGithubApp 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(app.id, uamiGithub.id, roleContributor)
  scope: app
  properties: {
    principalId: uamiGithub.properties.principalId
    roleDefinitionId: roleContributor
    principalType: 'ServicePrincipal'
  }
}

resource contributorGithubJob 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(migrateJob.id, uamiGithub.id, roleContributor)
  scope: migrateJob
  properties: {
    principalId: uamiGithub.properties.principalId
    roleDefinitionId: roleContributor
    principalType: 'ServicePrincipal'
  }
}

// ---------------------------------------------------------------------------
// Container App + migration Job
// ---------------------------------------------------------------------------

var appFqdn = '${appName}.${managedEnv.properties.defaultDomain}'
var kvUri = keyVault.properties.vaultUri

// Secrets pulled from Key Vault by the app identity.
var appSecrets = concat(
  [
    { name: 'database-url', keyVaultUrl: '${kvUri}secrets/database-url', identity: uamiApp.id }
    { name: 'session-secret', keyVaultUrl: '${kvUri}secrets/session-secret', identity: uamiApp.id }
  ],
  auth0Enabled ? [
    { name: 'auth0-client-secret', keyVaultUrl: '${kvUri}secrets/auth0-client-secret', identity: uamiApp.id }
  ] : []
)

var baseEnv = [
  { name: 'BIND_ADDRESS', value: '0.0.0.0:8080' }
  { name: 'BASE_URL', value: 'https://${appFqdn}' }
  { name: 'COOKIE_SECURE', value: 'true' }
  { name: 'STATIC_DIR', value: '/app/static' }
  { name: 'AUTH0_ROLES_CLAIM', value: 'https://amateur-radio-tools/roles' }
  { name: 'AUTH0_DOMAIN', value: auth0Domain }
  { name: 'AUTH0_CLIENT_ID', value: auth0ClientId }
  { name: 'DATABASE_URL', secretRef: 'database-url' }
  { name: 'SESSION_SECRET', secretRef: 'session-secret' }
]

var appEnv = auth0Enabled ? concat(baseEnv, [ { name: 'AUTH0_CLIENT_SECRET', secretRef: 'auth0-client-secret' } ]) : baseEnv

resource app 'Microsoft.App/containerApps@2024-03-01' = {
  name: appName
  location: location
  identity: {
    type: 'UserAssigned'
    userAssignedIdentities: { '${uamiApp.id}': {} }
  }
  properties: {
    managedEnvironmentId: managedEnv.id
    configuration: {
      activeRevisionsMode: 'Single'
      ingress: {
        external: true
        targetPort: 8080
        transport: 'auto'
        allowInsecure: false
        traffic: [ { latestRevision: true, weight: 100 } ]
      }
      registries: [
        { server: acr.properties.loginServer, identity: uamiApp.id }
      ]
      secrets: appSecrets
    }
    template: {
      containers: [
        {
          name: 'web'
          image: containerImage
          resources: { cpu: json('0.5'), memory: '1Gi' }
          env: appEnv
          probes: [
            { type: 'Liveness', httpGet: { path: '/health', port: 8080 }, periodSeconds: 30 }
            { type: 'Readiness', httpGet: { path: '/health', port: 8080 }, periodSeconds: 10 }
            { type: 'Startup', httpGet: { path: '/health', port: 8080 }, periodSeconds: 5, failureThreshold: 30 }
          ]
        }
      ]
      scale: { minReplicas: 1, maxReplicas: 3 }
    }
  }
  // RBAC must exist before the app resolves Key Vault secret references.
  dependsOn: [ kvSecretsUserApp, secretDatabaseUrl, secretSessionSecret ]
}

// Runs `migration up` before the app image is rolled (see deploy workflow), avoiding a
// multi-replica migration race. Reuses the app image and the same secrets/identity.
resource migrateJob 'Microsoft.App/jobs@2024-03-01' = {
  name: jobName
  location: location
  identity: {
    type: 'UserAssigned'
    userAssignedIdentities: { '${uamiApp.id}': {} }
  }
  properties: {
    environmentId: managedEnv.id
    configuration: {
      triggerType: 'Manual'
      replicaTimeout: 600
      replicaRetryLimit: 1
      manualTriggerConfig: { parallelism: 1, replicaCompletionCount: 1 }
      registries: [
        { server: acr.properties.loginServer, identity: uamiApp.id }
      ]
      secrets: [
        { name: 'database-url', keyVaultUrl: '${kvUri}secrets/database-url', identity: uamiApp.id }
      ]
    }
    template: {
      containers: [
        {
          name: 'migrate'
          image: containerImage
          command: [ '/app/migration' ]
          args: [ 'up' ]
          resources: { cpu: json('0.25'), memory: '0.5Gi' }
          env: [ { name: 'DATABASE_URL', secretRef: 'database-url' } ]
        }
      ]
    }
  }
  dependsOn: [ kvSecretsUserApp, secretDatabaseUrl ]
}

// ---------------------------------------------------------------------------
// Outputs (copy into GitHub repository variables for the deploy workflow)
// ---------------------------------------------------------------------------

output acrName string = acr.name
output acrLoginServer string = acr.properties.loginServer
output containerAppName string = app.name
output containerAppFqdn string = app.properties.configuration.ingress.fqdn
output migrationJobName string = migrateJob.name
output resourceGroupName string = resourceGroup().name
output keyVaultName string = keyVault.name
output postgresFqdn string = postgres.properties.fullyQualifiedDomainName
output githubIdentityClientId string = uamiGithub.properties.clientId
output appIdentityClientId string = uamiApp.properties.clientId
output tenantId string = subscription().tenantId
output subscriptionId string = subscription().subscriptionId
