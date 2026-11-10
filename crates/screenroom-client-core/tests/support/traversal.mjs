// Generous for the seed; finite even when every response is already cached.
export const TRAVERSAL_LIMITS = Object.freeze({ pages: 512, items: 10_000, timeoutMS: 120_000 });

export async function withDeadline(operation, cancel, timeoutMS = TRAVERSAL_LIMITS.timeoutMS) {
  const deadline = performance.now() + timeoutMS;
  let timer;
  const expired = () => {
    cancel();
    return new Error(`overall time budget exceeded (${timeoutMS}ms)`);
  };
  if (timeoutMS <= 0) throw expired();
  try {
    const result = await Promise.race([
      Promise.resolve().then(operation),
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          reject(new Error(`overall time budget exceeded (${timeoutMS}ms)`));
          cancel();
        }, timeoutMS);
      }),
    ]);
    if (performance.now() >= deadline) throw expired();
    return result;
  } finally { clearTimeout(timer); }
}

export async function collectPages(label, fetchPage, field, cancel, limits = TRAVERSAL_LIMITS) {
  return withDeadline(async () => {
    const tokens = new Set();
    const items = [];
    let token = '';
    for (let page = 0; page < limits.pages; page++) {
      if (tokens.has(token)) throw new Error(`${label}: repeated pagination token`);
      tokens.add(token);
      const response = await fetchPage(token);
      if (!Array.isArray(response[field]) || typeof response.next_page_token !== 'string') throw new Error(`${label}: invalid page response`);
      if (response[field].length > limits.items - items.length) throw new Error(`${label}: item budget exceeded (${limits.items})`);
      items.push(...response[field]);
      token = response.next_page_token;
      if (!token) return items;
    }
    throw new Error(`${label}: page budget exceeded (${limits.pages})`);
  }, cancel, limits.timeoutMS).catch(error => { throw new Error(`${label} traversal failed: ${error.message}`); });
}
