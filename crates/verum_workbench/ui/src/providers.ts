export type Provider = {
  id: string; name: string; executableAvailable: boolean; connectionId: string | null;
  capabilities: { reviewedTools: boolean; workspaceContext: boolean; modelSelection: boolean };
};

export function providerCatalog(value: unknown): Provider[] {
  const providers = (value as { providers?: unknown } | null)?.providers;
  if (!Array.isArray(providers)) throw new Error('Invalid provider catalog');
  const ids = new Set<string>();
  for (const provider of providers) {
    if (!provider || typeof provider.id !== 'string' || !provider.id || ids.has(provider.id) ||
        typeof provider.name !== 'string' || !provider.name || typeof provider.executableAvailable !== 'boolean' ||
        !(provider.connectionId === null || typeof provider.connectionId === 'string') ||
        ['reviewedTools', 'workspaceContext', 'modelSelection'].some(key => typeof provider.capabilities?.[key] !== 'boolean'))
      throw new Error('Invalid provider entry');
    ids.add(provider.id);
  }
  return providers;
}
