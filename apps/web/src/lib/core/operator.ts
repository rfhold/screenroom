import init, { OperatorClient } from '../../../../../target/client-core/wasm/screenroom_client_core';
import wasmUrl from '../../../../../target/client-core/wasm/screenroom_client_core_bg.wasm?url';
import { operatorConfig } from './operator-config';
let initialized: Promise<unknown> | undefined;
export async function newOperatorClient(): Promise<OperatorClient> {
  if (!operatorConfig.enabled || !operatorConfig.attachmentIdentity) throw new Error('Maintenance is unavailable');
  await (initialized ??= init({ module_or_path: wasmUrl }));
  return new OperatorClient({ endpoint: new URL(operatorConfig.prefix, location.origin).href, timeout_ms: 8000 });
}

export class ReadOwner {
  generation = 0;
  reset() { this.generation++; }
  ticket() { const generation = this.generation; return { current: () => generation === this.generation }; }
}
