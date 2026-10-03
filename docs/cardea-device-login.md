# Cardea broker login for Cowboy CLI

Install the native `cardea` binary on each computer running Cowboy CLI. Configure
the reserved `cardea` Authentication Provider with Cowboy's existing registered
Ed25519 client, trusted issuer, allowed subject, and local account mapping.
Activate a Cardea Worker supporting the versioned device identity extension.
Browser OIDC login remains available through the same provider.

```bash
cardea --help
cowboy login https://cowboy.example --cardea-profile cowboy --device-name hawk
```

Cowboy obtains the trusted Cardea issuer and client ID from its own Service,
then invokes `cardea auth login`. The CLI prints the device review URL and code
for a human to approve on desktop, phone, or iPad. Cardea never asks agents to
collect passwords. Each computer has an independent Cardea key and an
independent Cowboy key. Neither private key crosses into the other product.

For an existing browser-login credential, use a separate `--auth-state-dir`
(and the same state directory on subsequent client commands), or revoke the
previous Cowboy client first and remove its local credential. A login profile
cannot silently replace an existing credential of another origin or profile.
`COWBOY_CARDEA_PROFILE` is an alternative to the explicit login flag.

Cowboy saves its product key before requesting approval or exchanging evidence.
Retry the same login after interruption; it keeps that key and Cardea resumes
the pending registration. After login, the profile is retained privately with
the Cowboy credential. Later client requests automatically invoke `cardea auth
credential` when the Cowboy access credential expires. These refreshes perform
no human login. Cardea automatically rotates an overdue Cardea key before
minting identity evidence; Cowboy keeps its own independent product key.

The Cardea CLI writes short-lived evidence to a private file, with a signed
binding to Cowboy's public key. Cowboy reads and removes that file, and its
Service consumes the evidence through its authenticated Cardea backend client.
The Service validates the exact subject, client, public key, grant ID, schema,
and deadlines, then issues its own sender-constrained credential for at most
five minutes. It returns no long-lived product refresh bearer for this path.
One stable Cowboy device ID represents the Cardea grant, so refresh does not
allocate another device or bypass local device revocation/capacity.

Cardea authorization defaults to one year and key rotation to 30 days; both are
configured in Cardea. Rotation cannot extend the original authorization.
Expired, revoked, recovered, or disabled grants require fresh human approval.
A locally revoked Cowboy device cannot be revived by refreshing the same
Cardea grant; revoke the grant and register a new one. Losing the Cowboy private
key likewise requires a new registration because its binding cannot be silently
replaced. Existing access can survive a Cardea revocation for at most its
five-minute lifetime; Cowboy local revocation is checked at resource admission
and when captured operations revalidate their authority.

This authenticates an existing Cowboy account. Cowboy still owns permissions,
API/device revocation, capacity, and action approval. Login does not enable a
host Operator endpoint or grant deployment rights. Machine enrollment remains
a separate Cowboy protocol; a Cardea CLI login is not Machine registration.

The extension uses `/api/auth/device/cardea` (GET for public trusted-provider
configuration, POST for single-use identity exchange). Other Authentication
Providers cannot use it merely by presenting an arbitrary OIDC token. The
reserved Cardea provider requires Ed25519 `private_key_jwt` client authentication.
Release Cardea's endpoint and binary before activating this Cowboy consumer.
