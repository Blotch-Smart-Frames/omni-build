# @blotch/omni-build-release

Makes what a release publishes (the npm packages, an archive per platform for the GitHub release, and the Homebrew formula that installs those), checks that each installs as it should, and publishes the npm packages. The release workflow publishes the rest itself: the GitHub release with `gh`, the crate with Cargo, and the formula with git.

| Package                         | What it holds                                                                                               |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `@blotch/omni-build`            | The launcher, `packages/omni-build` compiled, with every platform's package as an optional dependency       |
| `@blotch/omni-build-<os>-<cpu>` | One per platform in `packages/omni-build/src/platforms.ts`: that platform's binary, with its `os` and `cpu` |

npm installs an optional dependency only on a machine its `os` and `cpu` match, so each machine gets its own binary and no other, as with esbuild, rspack and oxlint. The platform packages aren't in the repository: npm refuses to install a workspace whose `os` or `cpu` doesn't match the machine, and can't lock a dependency that isn't published. So `pack` makes them, and writes the launcher's `optionalDependencies`, at the version every package shares.

| Download                          | What it holds                                                                                                                                                                                        |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `omni-build-<Rust target>.zip`    | For Windows: a folder of the same name with `omni.exe` and the licence                                                                                                                               |
| `omni-build-<Rust target>.tar.gz` | For macOS and Linux: the same, with `omni`, executable                                                                                                                                               |
| `SHA256SUMS`                      | Each archive's SHA-256, as `sha256sum` writes them                                                                                                                                                   |
| `omni-build.rb`                   | Not on the release: the formula in the tap, [`Blotch-Smart-Frames/homebrew-tap`](https://github.com/Blotch-Smart-Frames/homebrew-tap), which installs the macOS and Linux archives by their SHA-256s |

The archives' names have no version, so `releases/latest/download/<archive>` always gets the newest, and cargo-binstall finds them by the template in `packages/cli/Cargo.toml`, which `archives.test.ts` checks against them.

## Run it

```sh
npm run build -w @blotch/omni-build    # pack needs the launcher compiled
node packages/release/src/cli.ts pack --binaries <dir> --out <dir>
node packages/release/src/cli.ts check --packages <dir>
node packages/release/src/cli.ts publish --packages <dir> --tag v<version>   # the release workflow's
node packages/release/src/cli.ts archive --binaries <dir> --out <dir>
node packages/release/src/cli.ts check-archive --archives <dir>
node packages/release/src/cli.ts formula --archives <dir> --out <file> [--url <url>]
```

**`pack`** takes the six release binaries as CI's build job uploads them, each in a folder named `omni-build-<Rust target>`, and writes a tarball per package, as `npm publish` takes them, with `packages.json` listing them.

**`check`** installs `@blotch/omni-build` from those tarballs, as a user would: with `npm install @blotch/omni-build` in an empty project, from a registry on this computer that has the packed packages and nothing else, with a cache and settings of its own. Then it checks that npm installed this machine's binary and no other platform's, that `omni --version` runs it, and that `omniBuild()`, the API a build tool imports, repackages a shell with it.

**`publish`** publishes those tarballs to npm as public packages: the platforms' packages first, and the launcher, which lists them, last. It leaves any version npm already has, so a release that stopped partway can be run again, and publishes a prerelease, such as `1.0.0-beta.1`, under `next` rather than `latest`. With `--tag`, it publishes only if the tag is `v` and the packages' version.

**`archive`** takes the same six binaries, and writes an archive per platform, with `SHA256SUMS` beside them. It writes the tar and zip formats itself, so the archives are the same whichever OS makes them.

**`check-archive`** does with this machine's archive what a user would: checks it against `SHA256SUMS`, unpacks it, with PowerShell's `Expand-Archive` on Windows and `tar` elsewhere, and runs `omni --version` from it.

**`formula`** writes the Homebrew formula for those archives, downloading them from the GitHub release of the version, or from `--url`. CI gives it a `file://` URL, installs the formula from a tap of its own, and runs `brew test`, `brew audit --strict` and `brew style` on it, on macOS and Linux.

CI packs and archives once, on Linux, whose file modes npm keeps (the binaries have to be executable once installed), and checks on each of the six OSes and CPUs.

The tool uses only Node's built-in modules, and Node runs its TypeScript as it is, so CI runs `check` without `npm ci`.

## Releasing

A release is a tag. `.github/workflows/release.yml` publishes what CI made and installed for the tagged commit to every channel, once a maintainer approves it in the `npm` environment:

1. **npm:** the packages, with `publish`. npm trusts the workflow, in the `npm` environment, to publish them (trusted publishing): it trades GitHub's OIDC token for one that may, and adds their provenance.
2. **GitHub:** a release for the tag, with notes listing the pull requests merged since the last, and the archives and `SHA256SUMS`, each archive with an attestation of where it came from. A version such as `1.0.0-beta.1` is marked a prerelease, as npm tags it `next`.
3. **crates.io:** the crate, with `cargo publish`. crates.io trusts the workflow in the `crates-io` environment, as npm does in `npm`.
4. **Homebrew:** the formula, pushed to the tap once the GitHub release has the archives it downloads, with a deploy key only the `homebrew` environment holds.

No npm or crates.io token is stored anywhere. Every step leaves what's published already as it is, so a release that stopped partway can be run again.

1. In a pull request, set the new version everywhere: `npm version <version> --workspaces --no-git-tag-version` for every `package.json` and `package-lock.json`, `[workspace.package] version` in `Cargo.toml` by hand, then `cargo update --workspace` for `Cargo.lock`.
2. Once it's merged, tag the merge commit `v<version>` and push the tag: `git tag v<version> <commit>`, `git push origin v<version>`.
3. The workflow waits for CI's run on that commit, then asks for approval. Approve it in the run's page, and it publishes.

CI keeps the packed packages and the archives for 7 days. To release a commit after that, run CI on master again, then re-run the release.

### Setting it up

This is done once, for each channel.

#### npm

npm can only trust a workflow with a package that already exists, so each package's first version is published by hand:

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

#### crates.io

The same way as npm: crates.io trusts a workflow only with a crate it has, so the first version is published by hand, from the tag of the release that's first to publish to crates.io, before that release is approved. The workflow then finds the version there, and leaves it.

1. In the repository's settings, add a `crates-io` environment that allows only tags matching `v*` to deploy, with no required reviewers: the npm packages' approval comes first.
2. Logged in to crates.io (`cargo login`, with a token that may publish new crates), from the release's tag:

   ```sh
   git checkout v<version>
   cargo publish --locked --package omni-build
   ```

3. On crates.io, in the crate's **Settings**, under **Trusted Publishing**, add GitHub: the `Blotch-Smart-Frames` owner, the `omni-build` repository, the `release.yml` workflow and the `crates-io` environment. Then delete the token from step 2.

#### Homebrew

The tap is a repository of its own, `Blotch-Smart-Frames/homebrew-tap`, which the workflow pushes the formula to with a deploy key: an SSH key that may write to that repository and no other.

1. Create the repository, with a README so it has a branch to push to:

   ```sh
   gh repo create Blotch-Smart-Frames/homebrew-tap --public --add-readme --description "Homebrew formulae for Blotch's tools, among them omni-build"
   ```

2. In this repository's settings, add a `homebrew` environment that allows only tags matching `v*` to deploy, with no required reviewers.
3. Make a key, give the tap its public half, with write access, and the environment its private half, then delete both files:

   ```sh
   ssh-keygen -t ed25519 -N '' -C 'omni-build release' -f homebrew-tap-key
   gh repo deploy-key add homebrew-tap-key.pub --repo Blotch-Smart-Frames/homebrew-tap --allow-write --title 'omni-build release'
   gh secret set HOMEBREW_TAP_DEPLOY_KEY --env homebrew --repo Blotch-Smart-Frames/omni-build < homebrew-tap-key
   rm homebrew-tap-key homebrew-tap-key.pub
   ```
