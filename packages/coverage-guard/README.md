# @omni-build/coverage-guard

Fails the coverage gate wherever 100% would pass over untested code. A coverage figure counts what any test ran, and only in the files it was told to measure, so it can read 100% when:

- **A module has no test of its own**, and another module's test happens to run it. The hero's test renders the desktop icons, so the icons read as covered whether or not anything checks them.
- **A module's own test covers only part of it**, and other tests run the rest.
- **The coverage settings leave a file out.** The docs site's `include` was once `src/**/*.ts`, which left out every `.tsx` component, and the report read 100%.
- **A comment tells the tool to skip code**: `v8 ignore`, `c8 ignore`, `istanbul ignore`, `node:coverage`, or Rust's `#[coverage(off)]` and `cfg(coverage)`.
- **A Rust file is never compiled into the tests**, because no `mod` declares it, so no figure counts it.

`npm run test:coverage` runs the guard after the tests, locally and in CI's `coverage` job.

## What it checks

For each workspace package, it lists the files git would commit (tracked, and new ones not ignored), then:

| Check                                                                     | Applies to                                  |
| ------------------------------------------------------------------------- | ------------------------------------------- |
| A test beside the module, named after it: `run.ts`'s is `run.test.ts`     | Every module                                |
| That test, run on its own and measuring only the module, covers all of it | Every module but `src/cli.ts`               |
| No coverage-ignore comment                                                | Every module, and every Rust file in `src/` |
| In the report of the cargo-llvm-cov run `npm run test:coverage` just made | Every Rust file in `src/`                   |

A module is a script (`.ts`, `.tsx`, `.js` and the rest) in one of the package's directories. Scripts at a package's root are tool configuration (`vitest.config.ts`, `docusaurus.config.ts`), anything in a `fixtures/` directory is test data, and declaration files hold only types.

A module's test runs with its package's vitest config, so its environment, setup files and aliases apply, and so does its coverage `exclude`: a module that excludes is reported as unmeasured.

`src/cli.ts` needs its test, but isn't measured on its own: in a package with a command it only wires `run.ts` to the process, and its test runs it as a child process, which V8 coverage in the test process can't see. So the exemption can't hide code, it may have at most 15 lines of code (the largest wiring there is has 10).

## Run it

```sh
npm run test:coverage                     # the tests, then the guard
npm run check -w @omni-build/coverage-guard  # the guard alone, after a test:coverage run
```

It runs as many modules' tests at once as the machine has cores, and takes seconds.

## Things to know

- **Each package's vitest thresholds stay**, at 100%: they fail a run before the guard starts, with the report that says where.
- **The tests run real vitest** in `fixtures/isolation`, a package with a module of each kind the guard tells apart. Its tests fail and fall short on purpose, so this package's own vitest config only looks for tests in `src/`.
- Like the other packages with a command, `cli.ts` only wires `run.ts` to the process, and the tests reach everything else in-process.
