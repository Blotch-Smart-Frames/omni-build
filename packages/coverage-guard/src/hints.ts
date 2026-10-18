// Comments that tell a coverage tool to skip code, which it then reports
// as covered or leaves out of the count. Either way the report reads 100%
// for code no test ran, so none is allowed: the code gets a test, or goes.
const SCRIPT_HINT =
  /\b(?:v8|c8|istanbul)\s+ignore\b|\bnode:coverage\s+(?:ignore|disable)\b/;

// Rust's: the coverage attribute and its old name, and code compiled or run
// only while cargo-llvm-cov is measuring, or only while it isn't
// (`cfg(not(coverage))`, `cfg!(coverage)`).
const RUST_HINT =
  /\bcoverage\s*\(\s*off\s*\)|\bno_coverage\b|\bcfg(?:_attr)?!?\s*\([^)]*\bcoverage(?:_nightly)?\b/;

export interface Hint {
  /** From 1. */
  line: number;
  text: string;
}

/** The lines of a source file that hide code from coverage. */
export const hints = (source: string, language: 'script' | 'rust'): Hint[] => {
  const pattern = language === 'rust' ? RUST_HINT : SCRIPT_HINT;
  return source
    .split(/\r?\n/)
    .flatMap((text, index) =>
      pattern.test(text) ? [{ line: index + 1, text: text.trim() }] : [],
    );
};
