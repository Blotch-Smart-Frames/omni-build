# @blotch/omni-build-release

Makes the npm packages a release publishes, checks that npm installs them as it should, and publishes them.

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
node packages/release/src/cli.ts publish --packages <dir> --tag v<version>   # the release workflow's
```

**`pack`** takes the six release binaries as CI's build job uploads them, each in a folder named `omni-build-<Rust target>`, and writes a tarball per package, as `npm publish` takes them, with `packages.json` listing them.

**`check`** installs `@blotch/omni-build` from those tarballs, as a user would: with `npm install @blotch/omni-build` in an empty project, from a registry on this computer that has the packed packages and nothing else, with a cache and settings of its own. Then it checks that npm installed this machine's binary and no other platform's, that `omni --version` runs it, and that `omniBuild()`, the API a build tool imports, repackages a shell with it.

**`publish`** publishes those tarballs to npm as public packages: the platforms' packages first, and the launcher, which lists them, last. It leaves any version npm already has, so a release that stopped partway can be run again, and publishes a prerelease, such as `1.0.0-beta.1`, under `next` rather than `latest`. With `--tag`, it publishes only if the tag is `v` and the packages' version.

CI packs once, on Linux, whose file modes npm keeps (the binaries have to be executable once installed), and checks on each of the six OSes and CPUs.

The tool uses only Node's built-in modules, and Node runs its TypeScript as it is, so CI runs `check` without `npm ci`.

## Releasing

A release is a tag. `.github/workflows/release.yml` publishes the packages CI packed and installed for the tagged commit, once a maintainer approves it in the `npm` environment. No npm token is stored anywhere. npm trusts that workflow to publish these packages (trusted publishing): it trades GitHub's OIDC token for one that may, and adds their provenance.

1. In a pull request, set the new version everywhere: `npm version <version> --workspaces --no-git-tag-version` for every `package.json` and `package-lock.json`, `[workspace.package] version` in `Cargo.toml` by hand, then `cargo update --workspace` for `Cargo.lock`.
2. Once it's merged, tag the merge commit `v<version>` and push the tag: `git tag v<version> <commit>`, `git push origin v<version>`.
3. The workflow waits for CI's run on that commit, then asks for approval. Approve it in the run's page, and it publishes.

CI keeps the packed packages for 7 days. To release a commit after that, run CI on master again, then re-run the release.

### Setting it up

This is done once, for all seven packages. npm can only trust a workflow with a package that already exists, so each package's first version is published by hand:

1. In the repository's settings, the `npm` environment has a maintainer as its required reviewer, and allows only tags matching `v*` to deploy.
2. Download the `npm-packages` artifact from a CI run on master, with `gh run download <run> --name npm-packages --dir npm-packages`. Then publish its tarballs, the platforms' first, logged in to npm as an owner of `@blotch`, with two-factor authentication:

   ```sh
   cd npm-packages
   for tarball in blotch-omni-build-*-*.tgz; do npm publish "./$tarball" --access public; done
   npm publish ./blotch-omni-build-<version>.tgz --access public
   ```

3. Shortly before the first release from the workflow, trust the workflow with each package, as only the `npm` environment may use it (npm 11.15.0 or later). npm expires a trusted publisher that hasn't published within 2 days of being added, and it then has to be deleted and added again.

   ```sh
   npm trust github <package> --repo Blotch-Smart-Frames/omni-build --file release.yml --env npm --allow-publish
   ```

4. Once the workflow has published a release, on npm's website, in each package's **Settings**, under **Publishing access**, choose **Require two-factor authentication and disallow tokens**.
