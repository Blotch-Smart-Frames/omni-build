# @blotch/omni-build-release

Makes the npm packages a release publishes, and checks that npm installs them as it should, without publishing anything.

| Package                         | What it holds                                                                                               |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `@blotch/omni-build`            | The launcher, `packages/omni-build` compiled, with every platform's package as an optional dependency       |
| `@blotch/omni-build-<os>-<cpu>` | One per platform in `packages/omni-build/src/platforms.ts`: that platform's binary, with its `os` and `cpu` |

npm installs an optional dependency only on a machine its `os` and `cpu` match, so each machine gets its own binary and no other, as with esbuild, rspack and oxlint. The platform packages aren't in the repository: npm refuses to install a workspace whose `os` or `cpu` doesn't match the machine, and can't lock a dependency that isn't published. So `pack` makes them, and writes the launcher's `optionalDependencies`, at the version every package shares.

## Run it

```sh
npm run build -w @blotch/omni-build    # pack needs the launcher compiled
node packages/release/src/cli.ts pack --binaries <dir> --out <dir>
node packages/release/src/cli.ts check --packages <dir>
```

**`pack`** takes the six release binaries as CI's build job uploads them, each in a folder named `omni-build-<Rust target>`, and writes a tarball per package, as `npm publish` takes them, with `packages.json` listing them.

**`check`** installs `@blotch/omni-build` from those tarballs, as a user would: with `npm install @blotch/omni-build` in an empty project, from a registry on this computer that has the packed packages and nothing else, with a cache and settings of its own. Then it checks that npm installed this machine's binary and no other platform's, that `omni --version` runs it, and that `omniBuild()`, the API a build tool imports, repackages a shell with it.

CI packs once, on Linux, whose file modes npm keeps (the binaries have to be executable once installed), and checks on each of the six OSes and CPUs. Publishing to npm comes with the release workflow.

The tool uses only Node's built-in modules, and Node runs its TypeScript as it is, so CI runs `check` without `npm ci`.
