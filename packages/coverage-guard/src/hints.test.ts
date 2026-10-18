import { describe, expect, it } from 'vitest';

import { hints } from './hints.ts';

describe('in scripts', () => {
  it.each([
    '/* v8 ignore next */',
    '/* v8 ignore start */',
    '// v8 ignore next 3',
    '/* v8 ignore next -- @preserve */',
    '/* c8 ignore next */',
    '/* istanbul ignore else */',
    '/* node:coverage ignore next */',
    '/* node:coverage disable */',
  ])('finds %s', (comment) => {
    expect(hints(`const a = 1;\n  ${comment}\nconst b = 2;`, 'script')).toEqual(
      [{ line: 2, text: comment }],
    );
  });

  it('finds every one, on Windows line endings too', () => {
    const source =
      '/* v8 ignore next */\r\na();\r\n/* c8 ignore next */\r\nb();';

    expect(hints(source, 'script').map(({ line }) => line)).toEqual([1, 3]);
  });

  it('leaves alone code that only mentions ignoring or coverage', () => {
    const source = [
      "const ignore = ['dist'];",
      '// The v8 provider measures coverage.',
      'const v8ignored = true;',
      '#[coverage(off)]',
    ].join('\n');

    expect(hints(source, 'script')).toEqual([]);
  });
});

describe('in Rust', () => {
  it.each([
    '#[coverage(off)]',
    '#![coverage(off)]',
    '#[cfg_attr(coverage_nightly, coverage(off))]',
    '#[no_coverage]',
    '#[cfg(not(coverage))]',
    '#[cfg(coverage)]',
    'if cfg!(coverage) {',
  ])('finds %s', (line) => {
    expect(hints(`fn main() {}\n${line}`, 'rust')).toEqual([
      { line: 2, text: line },
    ]);
  });

  it('leaves alone code that only mentions coverage', () => {
    const source = [
      'let coverage = 100;',
      '#[cfg(test)]',
      '// cargo llvm-cov measures coverage.',
      '/* v8 ignore next */',
    ].join('\n');

    expect(hints(source, 'rust')).toEqual([]);
  });
});
