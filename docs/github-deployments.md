# GitHub Dockerfile deployments

Create an Application and open its **Source** tab. Choose **GitHub repository**,
enter `owner/repository` or its `https://github.com/owner/repository` URL, select a
branch, and set the build context and Dockerfile path. Context is relative to the
repository root; Dockerfile is relative to that context. Save the source and use
**Test repository** to check access, then **Deploy** in Overview. Set the correct
container port in Configuration and runtime variables in Environment.

Public repositories need no token, subject to GitHub's anonymous API rate limit.
Private repositories require a fine-grained GitHub token with **Contents: Read**
for the selected repository. Token settings are browser owner/admin only and
write-only: values are encrypted under a service-specific authenticated context
and never returned to the browser. Blank token preserves a saved token; replacement
and removal are explicit. GitHub Enterprise, SSH credentials, recursive submodules
and build-secret/build-argument management are outside this version.

## Builds and rollback

Deployments snapshot repository settings, branch, Dockerfile/context, credentials,
port and environment. At build start Ferrix resolves the selected branch head,
records the full commit SHA, and downloads that exact commit's GitHub archive.
A later push cannot alter the commit being built. GitHub redirects are restricted
to HTTPS GitHub hosts; signed archive URLs and tokens are not logged.

The archive is staged in a private temporary workspace and repackaged as the
selected Docker context. Root `.dockerignore` rules are applied before upload,
including wildcard patterns, parent directories and later `!` re-inclusions.
A Dockerfile-specific `<Dockerfile>.dockerignore` takes precedence. The Dockerfile
and applicable ignore file are always included for the builder. Source archives
containing links or special files are rejected. Paths cannot escape the workspace.
Limits are 128 MiB compressed, 512 MiB expanded, 50,000 entries and 128 path levels;
ignore files are limited to 1 MiB/10,000 lines. Temporary disk must hold the archive,
extracted repository and filtered context. Cleanup happens on completion/cancellation.

Ferrix sends the filtered context through Bollard to Docker **BuildKit**, supporting
multi-stage Dockerfiles and BuildKit Dockerfile features. BuildKit must be enabled
on the Docker server. Build steps run on that server and consume its CPU/RAM/disk;
provide sufficient capacity for the project's toolchain. Build-time package access
must be public or provided by the Dockerfile's own setup. Ferrix's GitHub token,
webhook secret and runtime environment variables are not injected into build steps.

Build output is bounded in Deployment History, followed by existing container
activation phases. The known repository token is redacted from persisted build
output; Dockerfiles can still print their own secrets, so treat logs as sensitive.
GitHub builds have a twenty-minute total deploy deadline; registry image deploys
retain five minutes. A failed build preserves the active service. The resulting
image is tagged by service/deployment and activation uses its immutable image ID.

Deployment History displays repository, branch, commit and initiating user. A
push-triggered deploy also identifies the GitHub sender. Rollback uses the saved
immutable image and runtime snapshot, preserving commit metadata without fetching
GitHub, rebuilding or requiring the old repository token. The original image must
remain available. Successful images and Docker build cache are retained for reuse;
Docker image/cache cleanup must preserve images needed for rollback.

Switching Source back to **Docker image** disables auto-deploy and clears pending
pushes. The registry image saved in Configuration is used for the next deploy.
Already queued deployments retain their immutable source snapshots. Source edits
show saved changes pending until a deployment applies them.

## Push auto-deploy

1. Save a GitHub source and enable **Auto-deploy when this branch receives a push**.
2. Generate a webhook secret in Source. Identity verification must be fresh; use
   Security to verify your password/TOTP if requested. The secret is shown once;
   saving/rotating it does not reveal the repository token.
3. In GitHub repository **Settings → Webhooks**, add the displayed URL, choose
   `application/json`, paste the generated Secret, select push events and keep
   SSL verification enabled. Creating this hook is a repository-admin action;
   Ferrix's read-only Contents token does not need webhook write access.
4. Push to the configured branch. Review delivery status in GitHub and deployment
   progress/history in Ferrix.

GitHub must reach Ferrix over public HTTPS. A localhost development address cannot
receive GitHub events; use an HTTPS tunnel/public Ferrix origin for auto-deploy.
Manual deploy and repository access tests work locally.

The public endpoint requires HMAC-SHA256 over the original body through
`X-Hub-Signature-256` and validates GitHub event/delivery headers. Invalid signatures,
wrong repositories/branches and deleted branch pushes cannot queue builds. Ping
is supported. Delivery IDs and signed payload hashes are deduplicated for 30 days;
replaying the same body under a different delivery header does not rebuild it.
Bodies are limited to 2 MiB; larger deliveries are rejected and can be deployed
manually. No payload, token or secret is included in the webhook URL or stored in
delivery audit. Rotation invalidates the old secret.

Pushes persist as one latest pending request per application. A busy deployment is
allowed to finish, then the pending request deploys the latest branch head. This
coalesces rapid pushes and avoids reverting to an older out-of-order delivery.
Intermediate commits may not each be deployed. Duplicate signed deliveries do not
queue additional jobs. The configuring user's membership/ban/role is checked again
before queueing; loss of owner/admin permission pauses auto-deploy. The deployment
worker also checks permissions at execution, and normal outcome notifications apply.

Back up Ferrix metadata and `FERRIX_AUTH_KEY`: these retain encrypted source and
webhook settings. Restore credentials and network access before enabling pushes.

## API and agent access

Browser owner/admin endpoints:

- `GET/POST /api/v1/services/{id}/source`
- `POST /api/v1/services/{id}/source/test`
- `POST /api/v1/services/{id}/source/webhook-secret`

Save body: `mode` (`image` or `github`), `repository`, `branch`, `dockerfile`,
`context`, optional `token`, `clear_token`, and `auto_deploy`. Responses expose
only metadata plus token/webhook configured flags. Secret generation returns its
one-time value. The public signed endpoint is
`POST /api/v1/webhooks/github/{service_id}`; browser cookies/API keys do not replace
its signature.

An API key with deploy permission can queue the existing saved GitHub source using
`POST /services/{id}/deploy`. Organization/project boundaries remain enforced. API
keys cannot inspect/change source credentials or rotate webhook secrets. The image
agent example deliberately refuses existing GitHub-source applications instead of
silently switching or deploying the wrong source. Configuration's image field
does not switch source; use the Source tab explicitly.

## Verification

Rust/PostgreSQL tests cover encrypted private-token snapshots, strict path/repo
validation, signature/body tampering, branch/deletion/repository filters, delivery
and payload deduplication, pending push persistence, busy services, permission loss,
BuildKit builds, ignored context files, active image serving and rollback without
rebuild. Set `FERRIX_TEST_GITHUB_LIVE=1` with the own-profile test database to run the
optional read-only public GitHub archive test. Private-token request behavior is
covered with disposable tokens; testing a real private repository requires your
own token configured through Source.

The disposable browser runner covers Source settings, write-only tokens, one-time
webhook secrets, localhost guidance, commit/sender history, search and mobile
layout. Its temporary mode-0600 session cache reuses valid test sessions so growth
of the suite does not hit production login-attempt limits; revoked sessions fall
back to a real sign-in and the cache is removed with the isolated test environment.
