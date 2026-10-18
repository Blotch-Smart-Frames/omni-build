import { expect, it } from 'vitest';

import { half } from './half.ts';

// Only one side of the module's branch.
it('says a positive number is positive', () => {
  expect(half(1)).toBe('positive');
});
