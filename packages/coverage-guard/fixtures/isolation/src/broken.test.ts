import { expect, it } from 'vitest';

import { broken } from './broken.ts';

it('fails', () => {
  expect(broken()).toBe('working');
});
