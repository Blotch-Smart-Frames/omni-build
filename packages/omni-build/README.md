# @blotch/omni-build

Build, sign, install and ship iOS and Android apps with no native toolchain: no Xcode, no JDK, no Android SDK.

**Status:** Phase 0 of [the plan](https://github.com/Blotch-Smart-Frames/omni-build/issues/1), with Phase 1 begun.

```sh
npm install --save-dev @blotch/omni-build
npx omni --help
```

OmniBuild is a native binary written in Rust. This package is a small launcher: npm installs the binary for your OS and CPU from one of the `@blotch/omni-build-*` packages, and `omni` runs it.

To use a binary from somewhere else, for a platform without a prebuilt binary or one you built with `cargo install omni-build`, set `OMNI_BUILD_BINARY` to its path.

## From a build tool

A framework or build tool runs OmniBuild through the package's API, which hands over each event the binary reports, typed, and resolves with the command's result:

```ts
import { omniBuild } from '@blotch/omni-build';

const { out } = await omniBuild(
  ['repackage', 'shell.apk', '--config', 'app.json', '--out', 'app.apk'],
  { onEvent: (event) => console.log(event.message) },
);
```

It rejects with an `OmniBuildError`, with OmniBuild's reason, when the command fails. [Running from a build tool](https://github.com/Blotch-Smart-Frames/omni-build/blob/master/packages/docs/docs/build-tools.md) lists the events, and how to read them without Node, from `omni --json`.
