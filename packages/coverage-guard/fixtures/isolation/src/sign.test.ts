import { expect, it } from 'vitest';

import { sign } from './sign.ts';

it('tells negative numbers from positive ones', () => {
  expect(sign(-1)).toBe('negative');
  expect(sign(1)).toBe('positive');
});
