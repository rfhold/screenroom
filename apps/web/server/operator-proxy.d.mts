import type { IncomingMessage, ServerResponse } from 'node:http';
import type { Plugin } from 'vite';

export const OPERATOR_PREFIX: '/__operator/api';
export type OperatorServerConfig = Readonly<{ target: string; origin: string }>;
export function isLoopback(address: string | undefined): boolean;
export function readOperatorConfig(env?: Record<string, string | undefined>): OperatorServerConfig | null;
export function operatorBrowserConfig(config: OperatorServerConfig | null): Readonly<{
  enabled: boolean;
  prefix: '/__operator/api';
  attachmentIdentity: string | null;
}>;
export function createOperatorMiddleware(
  config: OperatorServerConfig | null,
  options?: { timeoutMs?: number; responseLimit?: number }
): (req: IncomingMessage, res: ServerResponse, next: () => void) => void;
export function operatorProxyPlugin(config: OperatorServerConfig | null): Plugin;
