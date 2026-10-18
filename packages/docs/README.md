# @omni-build/docs

OmniBuild's documentation site and landing page, built with [Docusaurus](https://docusaurus.io) and published to GitHub Pages at https://blotch-smart-frames.github.io/omni-build/.

```sh
npm start -w @omni-build/docs          # dev server with live reload
npm run build -w @omni-build/docs      # the production site, in build/
npm run serve -w @omni-build/docs      # serve build/ locally
npm test -w @omni-build/docs           # the .ts modules' tests; test:coverage fails below 100%
npm run typecheck -w @omni-build/docs  # tsc; Docusaurus itself only strips the types
```

The pages are in `docs/`. The landing page is `src/pages/index.tsx`, and its components are in `src/components/`.

## Publishing

`.github/workflows/docs.yml` builds and deploys the site from `master`. The CI gate's `docs` job builds it on every pull request, and a broken link or anchor fails the build.

## Things to know

- **No `"type": "module"`** in `package.json`. Docusaurus generates a route registry that uses webpack's `require.resolveWeak`, and the bundler only handles that in files it treats as CommonJS. The config files still use `import` and `export`: Docusaurus loads them either way.
- **The landing page's animations come from [React Bits](https://reactbits.dev)**, copied in as source, which is how React Bits is distributed. They're its TypeScript + CSS variants (TS-CSS), unchanged but for oxfmt's formatting, in `src/components/react-bits/`, renamed to kebab-case for the repository's lint rules, with React Bits' licence beside them. Updating one is a copy of the new TS-CSS variant. Its terms allow them in a website, but not redistributing the components themselves.
- **`tsconfig.json` copies `@docusaurus/tsconfig`'s options rather than extending it.** That package sets `baseUrl`, which TypeScript 7 removed.
- **Unit tests cover the `.ts` modules, at 100%**: each has a `.test.ts` beside it, run by vitest (with jsdom for the ones that touch the page), and CI's coverage gate fails below 100% of them. The components (`.tsx`), the content and the links are presentation, and the build, which renders every page and fails on a broken link or anchor, is their test.
- **`vitest.config.mts`, not `.ts`**, so it loads as an ES module in a package without `"type": "module"`. Vite warns that it will stop loading a module-syntax `.ts` config from such a package.
