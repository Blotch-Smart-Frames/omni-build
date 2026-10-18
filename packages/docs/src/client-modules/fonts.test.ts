import { expect, it, vi } from 'vitest';

// Each face stands in for its Fontsource package, recording that it was
// loaded, so the test sees which fonts the site gets without bundling CSS.
const loaded = vi.hoisted((): string[] => []);

vi.mock('@fontsource/jersey-10', () => {
  loaded.push('@fontsource/jersey-10');
  return {};
});
vi.mock('@fontsource/tiny5', () => {
  loaded.push('@fontsource/tiny5');
  return {};
});
vi.mock('@fontsource-variable/atkinson-hyperlegible-next', () => {
  loaded.push('@fontsource-variable/atkinson-hyperlegible-next');
  return {};
});

it("loads the site's three faces from Fontsource", async () => {
  await import('./fonts');

  expect(loaded).toEqual([
    '@fontsource/jersey-10',
    '@fontsource/tiny5',
    '@fontsource-variable/atkinson-hyperlegible-next',
  ]);
});
