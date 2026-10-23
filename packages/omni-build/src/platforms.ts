// The prebuilt binaries OmniBuild publishes, keyed by Node's
// `${process.platform}-${process.arch}`. Each becomes an npm package,
// `@blotch/omni-build-<key>`, holding the binary Cargo builds for the Rust
// target, with `os` and `cpu` fields so npm installs only the one matching
// the machine, as esbuild, rspack and oxlint do.
//
// The release tooling (Phase 1 in #1) generates those packages from this list
// and writes them into this package's optionalDependencies when it publishes,
// rather than the repository listing them: npm can't lock a version that
// isn't published yet, and `npm ci` fails on a dependency missing from the
// lockfile.
//
// Linux has one package per CPU, not one per libc: its binary is linked
// statically against musl, so it runs on glibc and musl distributions alike.
export const PLATFORMS: Record<string, string> = {
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-musl',
  'linux-x64': 'x86_64-unknown-linux-musl',
  'win32-arm64': 'aarch64-pc-windows-msvc',
  'win32-x64': 'x86_64-pc-windows-msvc',
};
