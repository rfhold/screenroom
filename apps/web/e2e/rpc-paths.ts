const services = {
  BrowseService: { namespace: 'catalog', methods: ['GetHome', 'ListLibraries', 'ListLibraryItems'] },
  CatalogService: { namespace: 'catalog', methods: ['GetCatalogInfo', 'GetItem', 'ListChildren', 'ListCredits', 'ListArtwork'] },
  SearchService: { namespace: 'catalog', methods: ['Search', 'GetSearchCapabilities'] },
  UserService: { namespace: 'user', methods: ['GetCurrentUser'] },
} as const;

export type RpcMethod = { [S in keyof typeof services]: `${S}/${typeof services[S]['methods'][number]}` }[keyof typeof services];

const paths = new Map<string, string>(Object.entries(services).flatMap(([service, { namespace, methods }]) =>
  methods.map(method => [`${service}/${method}`, `/screenroom.${namespace}.v1.${service}/${method}`] as const)));

export function rpcPath(method: RpcMethod): string {
  const result = paths.get(method);
  if (!result) throw new Error(`Unknown RPC: ${method}`);
  return result;
}

export const rpcRoute = (method: RpcMethod) => `**${rpcPath(method)}`;
export const isRpcPath = (pathname: string) => [...paths.values()].includes(pathname);
export const rpcRoutes = new RegExp(`^https?://[^/]+(?:${[...paths.values()].map(value => value.replaceAll('.', '\\.')).join('|')})$`);
