import { client } from './client';

export async function newOperatorClient() {
  return (await client()).operator();
}

export class ReadOwner {
  generation = 0;
  reset() { this.generation++; }
  ticket() { const generation = this.generation; return { current: () => generation === this.generation }; }
}
