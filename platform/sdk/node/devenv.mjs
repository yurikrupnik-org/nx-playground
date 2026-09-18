/**
 * Node SDK for DevEnvironment claims (platform.playground.io/v1alpha1).
 *
 * Provisions and inspects Crossplane DevEnvironment claims which spin up
 * postgres + redis + nats in a dedicated namespace.
 *
 * MUST run under node, not bun: see the runtime guard below.
 */
import {
  CoreV1Api,
  CustomObjectsApi,
  KubeConfig,
  PatchStrategy,
  setHeaderOptions,
} from '@kubernetes/client-node';

// Fail loudly at module load instead of with an opaque TLS error on the first
// request. Placed after the imports because ESM hoists them regardless of where
// this check is written.
if (typeof Bun !== 'undefined') {
  throw new Error(
    '@platform/devenv must run under node, not bun: @kubernetes/client-node ' +
      'uses a TLS agent bun does not implement (see platform/README.md).',
  );
}

const GROUP = 'platform.playground.io';
const VERSION = 'v1alpha1';
const PLURAL = 'devenvironments';
const KIND = 'DevEnvironment';
const FIELD_MANAGER = 'devenv-sdk';

let apis;

function api() {
  if (!apis) {
    const kc = new KubeConfig();
    kc.loadFromDefault();
    apis = {
      custom: kc.makeApiClient(CustomObjectsApi),
      core: kc.makeApiClient(CoreV1Api),
    };
  }
  return apis;
}

/**
 * Render the claim. Optional parameters are omitted when undefined so the XRD's
 * own defaults apply (see platform/dev-env/xrd.yaml).
 */
function claimBody(name, postgres, redis, nats, options = {}) {
  const { instances, storageGB, jetstream } = options;

  const pg = { enabled: Boolean(postgres) };
  if (postgres) {
    if (instances !== undefined) pg.instances = Number(instances);
    if (storageGB !== undefined) pg.storageGB = Number(storageGB);
  }

  const natsParams = { enabled: Boolean(nats) };
  if (nats && jetstream !== undefined)
    natsParams.jetstream = Boolean(jetstream);

  return {
    apiVersion: `${GROUP}/${VERSION}`,
    kind: KIND,
    metadata: { name },
    spec: {
      parameters: {
        postgres: pg,
        redis: { enabled: Boolean(redis) },
        nats: natsParams,
      },
    },
  };
}

/**
 * Server-side apply the DevEnvironment claim. Idempotent.
 *
 * `options.instances` (1-3) and `options.storageGB` (1-20) size postgres;
 * `options.jetstream` toggles NATS JetStream. Omit them to take the XRD defaults.
 */
export async function create(
  name,
  namespace = 'default',
  postgres = true,
  redis = true,
  nats = true,
  options = {},
) {
  const { custom } = api();
  return custom.patchNamespacedCustomObject(
    {
      group: GROUP,
      version: VERSION,
      namespace,
      plural: PLURAL,
      name,
      body: claimBody(name, postgres, redis, nats, options),
      fieldManager: FIELD_MANAGER,
      force: true,
    },
    setHeaderOptions('Content-Type', PatchStrategy.ServerSideApply),
  );
}

/**
 * Fetch the claim: { ready, environment, conditions }.
 * Tolerates a claim whose status has not been populated yet.
 */
export async function get(name, namespace = 'default') {
  const { custom } = api();
  const obj = await custom.getNamespacedCustomObject({
    group: GROUP,
    version: VERSION,
    namespace,
    plural: PLURAL,
    name,
  });
  const status = obj?.status ?? {};
  const conditions = status.conditions ?? [];
  const ready = conditions.some(
    (c) => c.type === 'Ready' && c.status === 'True',
  );
  return { ready, environment: status.environment ?? null, conditions };
}

/**
 * Resolve connection details for a ready environment.
 *
 * Reads the CNPG-format postgres secret from the environment namespace and
 * returns { postgres: {uri, username, password, host, port, dbname},
 * redis_host, nats_url }.
 */
export async function connection(name, namespace = 'default') {
  const { core } = api();
  const { environment: env } = await get(name, namespace);
  if (!env) {
    throw new Error(
      `DevEnvironment ${namespace}/${name} has no status.environment yet`,
    );
  }
  let postgres = null;
  if (env.postgresSecret && env.namespace) {
    const secret = await core.readNamespacedSecret({
      name: env.postgresSecret,
      namespace: env.namespace,
    });
    const data = Object.fromEntries(
      Object.entries(secret.data ?? {}).map(([k, v]) => [
        k,
        Buffer.from(v, 'base64').toString('utf-8'),
      ]),
    );
    postgres = {
      uri: data.uri ?? null,
      username: data.username ?? null,
      password: data.password ?? null,
      host: data.host ?? null,
      port: data.port ?? null,
      dbname: data.dbname ?? null,
    };
  }
  return {
    postgres,
    redis_host: env.redisHost ?? null,
    nats_url: env.natsUrl ?? null,
  };
}

/** Delete the DevEnvironment claim. */
export async function deleteEnvironment(name, namespace = 'default') {
  const { custom } = api();
  return custom.deleteNamespacedCustomObject({
    group: GROUP,
    version: VERSION,
    namespace,
    plural: PLURAL,
    name,
  });
}

export { deleteEnvironment as delete };
