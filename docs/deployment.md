# Pantheon deployment

[`infra/pulumi/`](../infra/pulumi/) defines the private backend and Web SPA for two Pantheon stacks. The local Compose workflow remains separate. This deployment excludes the static website and native packages.

| Stack | Namespace | HTTPS host | Pipeline |
| --- | --- | --- | --- |
| `preview` | `screenroom-preview` | `preview-screenroom.holdenitdown.net` | Push to `main` |
| `prod` | `screenroom` | `screenroom.holdenitdown.net` | Signed `vMAJOR.MINOR.PATCH` tag |

The [Tekton pipelines](../.tekton/) use the shared BuildKit, manifest, promotion, and Pulumi tasks. Preview builds six images in parallel for `amd64` and `arm64`, then deploys their digests together. Production promotes the same revision digests without a rebuild. The tag must name an ancestor of `main` and match the workspace version.

Use the shared Pulumi backend and deployment task. The task expects the stack state record to exist, as it does for sibling projects. For a manual preview, pass `--config` for each immutable image: `catalogImage`, `userImage`, `workerImage`, `playbackGatewayImage`, `indexOperatorImage`, and `webImage`.

The shared HTTPS Gateway terminates TLS and routes catalog, user, playback, and operator RPCs over h2c. The worker has no Gateway route. Envoy and `/__screenroom_instance` belong to local Compose.

The same host serves the [Web SPA](../apps/web/) at `/` and supports direct navigation to SPA routes. The browser uses its own origin for RPC and media requests. A mounted configuration file supplies the local user selector. The selector does not authenticate requests. The Web image serves static files only; it does not proxy operator traffic.

## Before activation

- Confirm that the Jellyfin Service selects the pods in `jellyfinPodLabels` and that workloads reach it on TCP 8096. Affinity prefers Jellyfin's node.
- Verify the Ceph storage classes, S3 endpoint, artwork permissions, backup operation, and recovery procedure.
- Verify that the PostgreSQL 18 system image supplies `vector`, `pg_trgm`, and Barman.
- Verify the Gateway listener, DNS, certificate, trusted-network boundary, and Gateway log settings. The backend accepts public-port traffic from the `ingress` namespace, like sibling apps. Verify native gRPC, gRPC-Web, artwork, and media range requests. No Authentik or application authentication protects the route. Do not expose it to an untrusted network.
- Confirm Pipelines as Code registration, signer custody, registry authorization, and Pulumi state privileges. The release verifier uses Homelab's General CI image and expects a `screenroom-release-trusted-signers` Secret with `signing-key.asc` in the CI namespace. Confirm serialized stack updates and stale-run handling. The shared Pulumi task skips preview, so inspect a manual stack preview before the first push or tag that triggers a deployment.
- Both stacks use the shared Homelab deploy task and its `tekton-cluster-kubeconfig`, like the sibling repositories.

## Automatic setup

| Resource | Setup |
| --- | --- |
| PostgreSQL | CloudNativePG creates the database and app Secret. Bootstrap SQL installs the extensions and grant roles. Managed roles use generated passwords for the five service logins. URLs verify TLS against the cluster CA. |
| Ceph | The backup claim supplies CloudNativePG's Secret. The worker uses the artwork claim's owner key. Catalog uses a separate claim's key, which the artwork bucket policy grants only object reads. Catalog's own claim bucket stays empty. |
| Private volumes | The operator and playback gateway initialize their own `0700` directories in init containers, as Waltr does. Jellyfin-facing readers mount the credential claim read-only. |
| Initial user | The user service runs `screenroom-user-init --create-missing` before it starts. The selector comes from the `screenroom-initial-user` ConfigMap. Repeated startup preserves changes to the user and their provider links. |

Setup does not import provider data or copy the local instance's credentials. Web Maintenance uses the same HTTPS origin as the rest of the app. Any caller that can reach this trusted private origin can administer the instance. Do not expose it to untrusted networks.

The deployment sets no cross-origin browser allowlist. If a separate Web origin enters scope, review an explicit origin allowlist first. Keep the media capability query parameter out of Gateway logs and traces.

## Embeddings

Both stacks configure catalog and the index worker to use Agent Gateway's `local-embedding` alias over HTTPS. They share the `qwen3-embedding-0.6b-local-v1` revision and the database's 1024 dimensions. Only those services gain egress on TCP 443 to the private Gateway VIP or its pods. The stacks do not set an embedding API key. See the [search contract](backend.md#search) for lexical fallback and model revision rules.

## Local verification

From `infra/pulumi/`, run `bun install --frozen-lockfile` and `bun run build`. The build type-checks the Pulumi program without contacting Kubernetes. It does not validate backup compatibility, media transport, or provider acceptance on Pantheon.
