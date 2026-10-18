import { expect, it } from 'vitest';

import { excluded } from './excluded.ts';

it('runs all of the module', () => {
  expect(excluded()).toBe('excluded');
});
