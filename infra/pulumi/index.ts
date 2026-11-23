import * as k8s from "@pulumi/kubernetes";
import * as pulumi from "@pulumi/pulumi";
import * as random from "@pulumi/random";

const config = new pulumi.Config();
const namespaceName = config.require("namespace");
const hostname = config.require("hostname");
const protectData = config.requireBoolean("protectData");

const origin = `https://${hostname}`;
const databaseImage = config.require("databaseImage");
const s3Endpoint = config.require("s3Endpoint");
const s3Region = config.require("s3Region");
const s3Peer = { ipBlock: { cidr: config.require("s3EgressCidr") } };
const components = {
  catalog: "catalogImage",
  user: "userImage",
  worker: "workerImage",
  "playback-gateway": "playbackGatewayImage",
  "index-operator": "indexOperatorImage",
} as const;
type Component = keyof typeof components;
const images = Object.fromEntries(Object.entries(components).map(([name, key]) => [
  name, config.require(key),
]));
const webImage = config.require("webImage");
const gatewayNamespace = "ingress";
const gatewayName = "default-gateway";
const labels = {
  "app.kubernetes.io/name": "screenroom",
  "app.kubernetes.io/instance": namespaceName,
  "app.kubernetes.io/part-of": "screenroom",
  "app.kubernetes.io/managed-by": "pulumi",
};
const namespace = new k8s.core.v1.Namespace("screenroom-namespace", {
  metadata: { name: namespaceName, labels },
});
const metadata = (name: string) => ({ name, namespace: namespace.metadata.name, labels });
const protection = { protect: protectData };

const databaseName = "screenroom";
const clusterName = "screenroom-postgres";
const postgresCaPath = "/var/run/secrets/screenroom/postgres/ca.crt";
const databaseUrl = (username: string, password: string) =>
  `postgresql://${encodeURIComponent(username)}:${encodeURIComponent(password)}@${clusterName}-rw.${namespaceName}.svc:5432/${databaseName}?sslmode=verify-full&sslrootcert=${encodeURIComponent(postgresCaPath)}`;
const runtimeRoles = {
  catalog: ["catalog_runtime", "screenroom_catalog"],
  user: ["user_runtime", "screenroom_user"],
  worker: ["worker_runtime", "screenroom_worker"],
  "playback-gateway": ["playback_runtime", "screenroom_playback"],
  "index-operator": ["operator_runtime", "screenroom_operator"],
} as const satisfies Record<Component, readonly [string, string]>;

function databaseSecret(component: string, username: string) {
  const name = `screenroom-db-${component}`;
  const password = pulumi.secret(new random.RandomPassword(name, { length: 48, special: false }, protection).result);
  return new k8s.core.v1.Secret(name, {
    metadata: metadata(name), type: "kubernetes.io/basic-auth",
    stringData: {
      username, password,
      "database-url": password.apply(value => databaseUrl(username, value)),
    },
  }, protection);
}

const runtimeSecrets = Object.fromEntries(Object.entries(runtimeRoles).map(([component, [login]]) => [
  component, databaseSecret(component, login),
]));

function bucket(purpose: string, named?: { bucketName: pulumi.Input<string>; bucketPolicy: pulumi.Input<string> }) {
  return new k8s.apiextensions.CustomResource(`screenroom-${purpose}-bucket`, {
    apiVersion: "objectbucket.io/v1alpha1", kind: "ObjectBucketClaim",
    metadata: { ...metadata(`screenroom-${purpose}`),
      annotations: { "pulumi.com/waitFor": "jsonpath={.status.phase}=Bound" } },
    spec: {
      storageClassName: config.require("bucketStorageClass"),
      ...(named ? { bucketName: named.bucketName, additionalConfig: { bucketPolicy: named.bucketPolicy } }
        : { generateBucketName: `${namespaceName}-${purpose}` }),
    },
  }, protection);
}

const artworkId = new random.RandomId("screenroom-artwork-id", { byteLength: 8 }, protection).hex;
const artworkName = artworkId.apply(id => `${namespaceName}-artwork-${id}`);
const artworkReader = bucket("artwork-reader");
// Rook includes the claim UID in the generated principal. The reader's own bucket stays empty.
const readerPrincipal = artworkReader.metadata.apply(({ namespace, name, uid }) => {
  if (!namespace || !name || !uid) throw new Error("artwork reader claim is missing metadata");
  return `obc-${namespace}-${name}-${uid}`;
});
const artworkPolicy = pulumi.all([artworkName, readerPrincipal]).apply(([name, reader]) =>
  JSON.stringify({
    Version: "2012-10-17",
    Statement: [
      { Effect: "Allow", Principal: { AWS: `arn:aws:iam:::user/${reader}` },
        Action: ["s3:GetObject"], Resource: `arn:aws:s3:::${name}/*` },
    ],
  }));
const artworkBucket = bucket("artwork", { bucketName: artworkName, bucketPolicy: artworkPolicy });
const backupBucket = bucket("backups");
const backupConfig = pulumi.all([namespace.metadata.name, backupBucket.id]).apply(([ns]) =>
  k8s.core.v1.ConfigMap.get("screenroom-backups-generated-config", `${ns}/screenroom-backups`));
const databaseCluster = new k8s.apiextensions.CustomResource("screenroom-postgres", {
  apiVersion: "postgresql.cnpg.io/v1", kind: "Cluster",
  metadata: { ...metadata(clusterName), annotations: { "pulumi.com/waitFor": "condition=Ready" } },
  spec: {
    imageName: databaseImage,
    instances: 1,
    enableSuperuserAccess: false,
    storage: { size: config.require("databaseStorageSize"), storageClass: config.require("storageClass") },
    bootstrap: {
      initdb: {
        database: databaseName, owner: "app",
        postInitApplicationSQL: [
          "CREATE EXTENSION IF NOT EXISTS vector",
          "CREATE EXTENSION IF NOT EXISTS pg_trgm",
          ...Object.values(runtimeRoles).map(([, group]) =>
            `CREATE ROLE ${group} NOLOGIN${group === "screenroom_playback" ? " NOINHERIT" : ""}`),
          // The managed-connections migration grants catalog to operator. Give the
          // migration owner the needed admin option without broader role creation.
          "GRANT screenroom_catalog TO app WITH ADMIN TRUE, INHERIT FALSE, SET FALSE",
        ],
      },
    },
    managed: { roles: Object.entries(runtimeRoles).map(([component, [name, group]]) => ({
      name, ensure: "present", login: true, inherit: true, inRoles: [group],
      passwordSecret: { name: runtimeSecrets[component].metadata.name },
    })) },
    backup: {
      retentionPolicy: config.require("backupRetention"),
      barmanObjectStore: {
        destinationPath: backupConfig.data.apply((values) => `s3://${values.BUCKET_NAME}/database`),
        endpointURL: s3Endpoint,
        s3Credentials: {
          accessKeyId: { name: "screenroom-backups", key: "AWS_ACCESS_KEY_ID" },
          secretAccessKey: { name: "screenroom-backups", key: "AWS_SECRET_ACCESS_KEY" },
        },
        wal: { compression: "gzip" },
        data: { compression: "gzip" },
      },
    },
    resources: { requests: { cpu: "250m", memory: "512Mi" }, limits: { memory: "2Gi" } },
  },
}, { ...protection, dependsOn: [backupBucket] });
const databaseApp = databaseCluster.id.apply(() =>
  k8s.core.v1.Secret.get("screenroom-postgres-generated-app", `${namespaceName}/${clusterName}-app`));
const migrationUrl = pulumi.secret(databaseApp.data.apply(({ username, password }) =>
  databaseUrl(Buffer.from(username, "base64").toString(), Buffer.from(password, "base64").toString())));
const migrationSecret = new k8s.core.v1.Secret("screenroom-db-migration", {
  metadata: metadata("screenroom-db-migration"),
  stringData: { "database-url": migrationUrl },
});
new k8s.apiextensions.CustomResource("screenroom-postgres-backup", {
  apiVersion: "postgresql.cnpg.io/v1", kind: "ScheduledBackup",
  metadata: metadata("screenroom-postgres"),
  spec: {
    schedule: config.require("backupSchedule"),
    cluster: { name: "screenroom-postgres" },
    backupOwnerReference: "self",
    method: "barmanObjectStore",
  },
}, { dependsOn: [databaseCluster] });

function claim(name: string, accessMode: string) {
  return new k8s.core.v1.PersistentVolumeClaim(name, {
    metadata: metadata(name),
    spec: {
      accessModes: [accessMode], storageClassName: config.require("credentialStorageClass"),
      resources: { requests: { storage: "1Gi" } },
    },
  }, protection);
}

const credentials = claim("screenroom-jellyfin-credentials", "ReadWriteMany");
const playbackSecrets = claim("screenroom-playback-secrets", "ReadWriteOnce");

type Env = k8s.types.input.core.v1.EnvVar;
type Port = k8s.types.input.core.v1.ContainerPort;
type Rule = k8s.types.input.networking.v1.NetworkPolicyEgressRule;
const env = (name: string, value: string): Env => ({ name, value });
const embeddingEnv = [
  env("SCREENROOM_EMBEDDING_BASE_URL", config.require("embeddingBaseUrl")),
  env("SCREENROOM_EMBEDDING_MODEL", config.require("embeddingModel")),
  env("SCREENROOM_EMBEDDING_DIMENSIONS", "1024"),
  env("SCREENROOM_EMBEDDING_REVISION", config.require("embeddingRevision")),
];
const secretEnv = (name: string, secret: pulumi.Input<string>, key: string): Env =>
  ({ name, valueFrom: { secretKeyRef: { name: secret, key } } });
const dbEnv = (component: Component): Env[] => [
  { name: "DATABASE_URL", valueFrom: { secretKeyRef: {
    name: runtimeSecrets[component].metadata.name, key: "database-url",
  } } },
  { name: "SCREENROOM_MIGRATION_DATABASE_URL", valueFrom: { secretKeyRef: {
    name: migrationSecret.metadata.name, key: "database-url",
  } } },
];
const s3Env = (secret: pulumi.Input<string>): Env[] => [
  env("SCREENROOM_S3_ENDPOINT", s3Endpoint),
  env("SCREENROOM_S3_REGION", s3Region),
  { name: "SCREENROOM_S3_BUCKET", valueFrom: { configMapKeyRef: {
    name: artworkBucket.metadata.name, key: "BUCKET_NAME",
  } } },
  secretEnv("SCREENROOM_S3_ACCESS_KEY_ID", secret, "AWS_ACCESS_KEY_ID"),
  secretEnv("SCREENROOM_S3_SECRET_ACCESS_KEY", secret, "AWS_SECRET_ACCESS_KEY"),
];
const componentLabels = (component: string) => ({ ...labels, "app.kubernetes.io/component": component });
const nsSelector = (name: string) => ({ matchLabels: { "kubernetes.io/metadata.name": name } });
const pod = (component: string) => ({ podSelector: { matchLabels: componentLabels(component) } });
const port = (value: number) => [{ port: value, protocol: "TCP" }];
const dns: Rule = { to: [{ namespaceSelector: nsSelector("kube-system") }],
  ports: [{ port: 53, protocol: "UDP" }, { port: 53, protocol: "TCP" }] };
const postgres: Rule = { to: [{ podSelector: { matchLabels: { "cnpg.io/cluster": clusterName } } }],
  ports: port(5432) };
const s3: Rule = { to: [s3Peer], ports: port(443) };
const jellyfinPort = 8096;
const jellyfinNamespace = "media-server";
const credentialPath = "/run/screenroom/credentials";
const playbackPath = "/run/screenroom/playback";
const appSecurity = { allowPrivilegeEscalation: false, capabilities: { drop: ["ALL"] } };
const podSecurity = { runAsNonRoot: true, runAsUser: 10001, runAsGroup: 10001,
  seccompProfile: { type: "RuntimeDefault" } };
const initialUser = new random.RandomUuid("screenroom-initial-user", {}, protection).result;
const bootstrapUserConfig = new k8s.core.v1.ConfigMap("screenroom-initial-user", {
  metadata: metadata("screenroom-initial-user"),
  data: { "users.json": initialUser.apply(id => JSON.stringify([{ id, display_name: "Local user" }])) },
});

function deployBackend() {
  const jellyfinPodLabels = config.requireObject<Record<string, string>>("jellyfinPodLabels");
  const jellyfin: Rule = { to: [{
    namespaceSelector: nsSelector(jellyfinNamespace), podSelector: { matchLabels: jellyfinPodLabels },
  }], ports: port(jellyfinPort) };
  const affinity = { podAffinity: { preferredDuringSchedulingIgnoredDuringExecution: [{
    weight: 100,
    podAffinityTerm: { namespaces: [jellyfinNamespace], labelSelector: { matchLabels: jellyfinPodLabels },
      topologyKey: "kubernetes.io/hostname" },
  }] } };

  function service(name: string, ports: Port[], appProtocol?: string) {
    return new k8s.core.v1.Service(`${name}-service`, {
      metadata: metadata(name),
      spec: { type: "ClusterIP", selector: componentLabels(name),
        ports: ports.map(({ name, containerPort }) => ({
          name, port: containerPort, targetPort: name, appProtocol: name === "internal" ? undefined : appProtocol,
        })) },
    });
  }

  function network(name: string, ingress: k8s.types.input.networking.v1.NetworkPolicyIngressRule[], egress: Rule[]) {
    return new k8s.networking.v1.NetworkPolicy(`${name}-network`, {
      metadata: metadata(`screenroom-${name}`),
      spec: { podSelector: { matchLabels: componentLabels(name) }, policyTypes: ["Ingress", "Egress"], ingress,
        egress: [dns, ...egress] },
    });
  }

  function backend(name: Component, command: string[], vars: Env[], ports: Port[], egress: Rule[],
    ingress: k8s.types.input.networking.v1.NetworkPolicyIngressRule[] = [],
    mounts: { credentials?: "read" | "write"; playback?: boolean } = {}) {
    const volumes: k8s.types.input.core.v1.Volume[] = [{
      name: "postgres-ca", secret: { secretName: `${clusterName}-ca`, defaultMode: 0o444,
        items: [{ key: "ca.crt", path: "ca.crt" }] },
    }];
    const volumeMounts: k8s.types.input.core.v1.VolumeMount[] = [{
      name: "postgres-ca", mountPath: "/var/run/secrets/screenroom/postgres", readOnly: true,
    }];
    const initContainers: k8s.types.input.core.v1.Container[] = [];
    const dependencies: pulumi.Resource[] = [databaseCluster];
    const initializeDirectory = (volumeName: string, path: string) => {
      initContainers.push({
        name: `initialize-${volumeName}`, image: images[name], securityContext: appSecurity,
        command: ["/bin/sh", "-ec", `test ! -L "$1"
if [ ! -e "$1" ]; then
  install -d -m 0700 "$1"
  chmod 00700 "$1"
fi
test -d "$1"
test "$(stat -c %u:%g:%a "$1")" = 10001:10001:700
`, "initialize-directory", `${path}/private`],
        volumeMounts: [{ name: volumeName, mountPath: path }],
      });
    };
    if (name === "catalog" || name === "worker") dependencies.push(artworkBucket);
    if (mounts.credentials) {
      const readOnly = mounts.credentials === "read";
      volumes.push({ name: "credentials", persistentVolumeClaim: { claimName: credentials.metadata.name, readOnly } });
      volumeMounts.push({ name: "credentials", mountPath: credentialPath, readOnly });
      vars.push(env("SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY", `${credentialPath}/private`));
      if (!readOnly) initializeDirectory("credentials", credentialPath);
    }
    if (mounts.playback) {
      volumes.push({ name: "playback", persistentVolumeClaim: { claimName: playbackSecrets.metadata.name } });
      volumeMounts.push({ name: "playback", mountPath: playbackPath });
      vars.push(env("SCREENROOM_PLAYBACK_SECRET_DIRECTORY", `${playbackPath}/private`));
      initializeDirectory("playback", playbackPath);
    }
    if (name === "user") {
      volumes.push({ name: "users", configMap: { name: bootstrapUserConfig.metadata.name } });
      initContainers.push({
        name: "initialize-user", image: images.user, securityContext: appSecurity, env: dbEnv("user"),
        command: ["screenroom-user-init", "--create-missing", "/bootstrap/users.json"],
        volumeMounts: [volumeMounts[0], { name: "users", mountPath: "/bootstrap", readOnly: true }],
      });
    }
    const policy = network(name, ingress, [postgres, ...egress]);
    const playback = !!mounts.playback;
    new k8s.apps.v1.Deployment(name, {
      metadata: metadata(name),
      spec: {
        replicas: 1, strategy: { type: "Recreate" }, selector: { matchLabels: componentLabels(name) },
        template: { metadata: { labels: componentLabels(name) }, spec: {
          affinity: mounts.credentials ? affinity : undefined,
          automountServiceAccountToken: false,
          securityContext: mounts.credentials === "write" || playback
            ? { ...podSecurity, fsGroup: 10001, fsGroupChangePolicy: "OnRootMismatch" } : podSecurity,
          terminationGracePeriodSeconds: playback ? 75 : undefined,
          volumes,
          initContainers,
          containers: [{
            name, image: images[name], command, ports, env: [...dbEnv(name), ...vars], volumeMounts,
            securityContext: appSecurity,
            resources: { requests: { cpu: "100m", memory: "128Mi" }, limits: { memory: "1Gi" } },
            readinessProbe: playback ? { httpGet: { path: "/ready", port: 50054 } }
              : ports.length ? { tcpSocket: { port: ports[0].containerPort } } : undefined,
          }],
        } },
      },
    }, { dependsOn: [...dependencies, policy] });
    return ports.length ? service(name, ports, "kubernetes.io/h2c") : undefined;
  }

  const gateway = { namespaceSelector: nsSelector(gatewayNamespace) };
  const gatewayPod = { ...gateway, podSelector: { matchLabels: {
    "gateway.networking.k8s.io/gateway-name": gatewayName,
  } } };
  const embedding: Rule = {
    to: [{ ipBlock: { cidr: config.require("embeddingEgressCidr") } }, gatewayPod],
    ports: port(443),
  };
  const fromGateway = (value: number) => ({ from: [gateway], ports: port(value) });
  const listen = (name: string, value: number) => env(name, `0.0.0.0:${value}`);
  const catalog = backend("catalog", ["screenroom-catalog"], [
    listen("SCREENROOM_CATALOG_LISTEN", 50052), env("SCREENROOM_CATALOG_PUBLIC_ORIGIN", origin),
    env("SCREENROOM_USER_PRIVATE_URI", "http://user:50063"), ...s3Env(artworkReader.metadata.name), ...embeddingEnv,
  ], [{ name: "grpc", containerPort: 50052 }], [s3, embedding, { to: [pod("user")], ports: port(50063) }],
  [fromGateway(50052)]);
  const user = backend("user", ["screenroom-user"], [
    listen("SCREENROOM_USER_LISTEN", 50053), listen("SCREENROOM_USER_PRIVATE_LISTEN", 50063),
  ], [{ name: "grpc", containerPort: 50053 }, { name: "internal", containerPort: 50063 }], [jellyfin], [
    fromGateway(50053), { from: [pod("catalog")], ports: port(50063) },
  ], { credentials: "read" });
  backend("worker", ["screenroom-index-worker", "serve"], [
    ...s3Env(artworkBucket.metadata.name), ...embeddingEnv,
  ], [], [s3, jellyfin, embedding], [], { credentials: "read" });
  const playback = backend("playback-gateway", ["screenroom-playback-gateway"], [
    listen("SCREENROOM_PLAYBACK_GATEWAY_LISTEN", 50054), env("SCREENROOM_PLAYBACK_API_ORIGIN", origin),
  ], [{ name: "http", containerPort: 50054 }], [jellyfin], [fromGateway(50054)],
  { credentials: "read", playback: true });
  const operator = backend("index-operator", ["screenroom-index-operator"], [
    listen("SCREENROOM_OPERATOR_LISTEN", 50072),
    env("SCREENROOM_OPERATOR_CORS_ORIGINS", origin),
  ], [{ name: "grpc", containerPort: 50072 }], [jellyfin],
  [{ from: [gatewayPod], ports: port(50072) }], { credentials: "write" });

  const webPort = { name: "http", containerPort: 8080 };
  const webConfig = new k8s.core.v1.ConfigMap("screenroom-web-config", {
    metadata: metadata("screenroom-web-config"),
    data: { "screenroom-config.js": initialUser.apply(id => `window.__SCREENROOM_USER_ID__ = ${JSON.stringify(id)};\n`) },
  });
  const webPolicy = network("web", [{
    from: [gatewayPod],
    ports: port(webPort.containerPort),
  }], []);
  const webLabels = componentLabels("web");
  new k8s.apps.v1.Deployment("web", {
    metadata: metadata("web"),
    spec: {
      replicas: 1, selector: { matchLabels: webLabels },
      template: {
        metadata: { labels: webLabels },
        spec: {
          automountServiceAccountToken: false, securityContext: podSecurity,
          volumes: [{ name: "config", configMap: { name: webConfig.metadata.name } }],
          containers: [{
            name: "web", image: webImage, ports: [webPort], securityContext: appSecurity,
            volumeMounts: [{ name: "config", mountPath: "/etc/screenroom/web", readOnly: true }],
            readinessProbe: { httpGet: { path: "/screenroom-config.js", port: "http" } },
          }],
        },
      },
    },
  }, { dependsOn: [webPolicy] });
  const web = service("web", [webPort]);

  const match = (value: string) => ({ path: { type: "PathPrefix", value } });
  new k8s.apiextensions.CustomResource("screenroom-route", {
    apiVersion: "gateway.networking.k8s.io/v1", kind: "HTTPRoute",
    metadata: metadata("screenroom"),
    spec: {
      parentRefs: [{ group: "gateway.networking.k8s.io", kind: "Gateway", namespace: gatewayNamespace,
        name: gatewayName, sectionName: "https-0" }],
      hostnames: [hostname],
      rules: [
        {
          matches: ["/screenroom.catalog.v1.BrowseService", "/screenroom.catalog.v1.CatalogService",
            "/screenroom.catalog.v1.SearchService", "/artwork"].map(match),
          backendRefs: [{ name: catalog!.metadata.name, port: 50052 }],
          timeouts: { request: "60s" },
        },
        {
          matches: [match("/screenroom.user.v1.UserService")],
          backendRefs: [{ name: user!.metadata.name, port: 50053 }],
          timeouts: { request: "60s" },
        },
        {
          matches: [match("/screenroom.operator.v1.OperatorService")],
          backendRefs: [{ name: operator!.metadata.name, port: 50072 }],
          timeouts: { request: "30s" },
        },
        {
          matches: [match("/screenroom.playback_gateway.v1.PlaybackService")],
          backendRefs: [{ name: playback!.metadata.name, port: 50054 }],
          timeouts: { request: "35s" },
        },
        {
          matches: ["GET", "HEAD", "OPTIONS"].map(method => ({ ...match("/playback/v1/media"), method })),
          backendRefs: [{ name: playback!.metadata.name, port: 50054 }],
          timeouts: { request: "0s" },
        },
        {
          matches: ["GET", "HEAD"].map(method => ({ ...match("/"), method })),
          backendRefs: [{ name: web.metadata.name, port: webPort.containerPort }],
        },
      ],
    },
  });
}

deployBackend();

export const namespaceNameOutput = namespace.metadata.name;
export const originOutput = origin;
