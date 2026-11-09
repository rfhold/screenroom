export const categories = ['happy-path', 'retry', 'regression', 'all'];

export function selectedCategory() {
  const category = process.env.E2E_CATEGORY ?? 'all';
  if (!categories.includes(category)) {
    throw new Error(`Invalid E2E_CATEGORY ${JSON.stringify(category)}. Choose ${categories.join(', ')}.`);
  }
  return category;
}
