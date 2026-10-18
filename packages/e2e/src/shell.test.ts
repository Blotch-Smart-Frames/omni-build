import { describe, expect, it } from 'vitest';

import { SHELL, differences } from './shell.ts';

describe('differences', () => {
  it('is empty when the report has every expected value', () => {
    // Values the harness doesn't expect, such as the app's own extras, are
    // fine.
    expect(differences(SHELL, { ...SHELL, extra: 'value' })).toEqual([]);
  });

  it('describes each expected value that differs or is missing', () => {
    expect(
      differences(
        { id: 'dev.omnibuild.shell', name: 'Shell', build: '1' },
        { id: 'dev.omnibuild.shell', name: 'Other' },
      ),
    ).toEqual([
      'name: expected "Shell", got "Other"',
      'build: expected "1", got null',
    ]);
  });
});
