# @blotch/omni-build-docs

OmniBuild's documentation site and landing page, built with [Docusaurus](https://docusaurus.io) and published to GitHub Pages at https://blotch-smart-frames.github.io/omni-build/.

```sh
npm start -w @blotch/omni-build-docs          # dev server with live reload
npm run build -w @blotch/omni-build-docs      # the production site, in build/
npm run serve -w @blotch/omni-build-docs      # serve build/ locally
npm test -w @blotch/omni-build-docs           # every module's tests; test:coverage fails below 100%
npm run typecheck -w @blotch/omni-build-docs  # tsc; Docusaurus itself only strips the types
```

The pages are in `docs/`. The landing page is `src/pages/index.tsx`, and its components are in `src/components/`.

## Publishing

`.github/workflows/docs.yml` builds and deploys the site from `master`. The CI gate's `docs` job builds it on every pull request, and a broken link or anchor fails the build.

## Things to know

- **No `"type": "module"`** in `package.json`. Docusaurus generates a route registry that uses webpack's `require.resolveWeak`, and the bundler only handles that in files it treats as CommonJS. The config files still use `import` and `export`: Docusaurus loads them either way.
- **The landing page's animations come from [React Bits](https://reactbits.dev)**, copied in as source, which is how React Bits is distributed. They're its TypeScript + CSS variants (TS-CSS), unchanged but for oxfmt's formatting, in `src/components/react-bits/`, renamed to kebab-case for the repository's lint rules, with React Bits' licence beside them. Updating one is a copy of the new TS-CSS variant. Its terms allow them in a website, but not redistributing the components themselves.
- **`tsconfig.json` copies `@docusaurus/tsconfig`'s options rather than extending it.** That package sets `baseUrl`, which TypeScript 7 removed.
- **Every module has a test beside it, at 100%**: the components (`.tsx`) a `.test.tsx` with [React Testing Library](https://testing-library.com/docs/react-testing-library/intro/), the rest a `.test.ts`, all run by vitest in jsdom. Each test covers all of its own module: coverage-guard (`packages/coverage-guard`) runs it alone and fails CI when it doesn't, so the hero's test rendering a button doesn't count as the button's. The build, which renders every page and fails on a broken link or anchor, tests the content and the links.
- **The tests mock what Docusaurus provides at build time**: `@docusaurus/Link`, `@docusaurus/BrowserOnly`, `@theme/Layout` and `useColorMode`, which need a running site. `vitest.config.mts` aliases them to the files Docusaurus does, so vitest can resolve them for `vi.mock`.
- **The React Bits tests stand in for the browser APIs jsdom lacks**: WebGL 2, canvas 2D, `Path2D`, animation frames and the observers. Their few odd stubs (a ref React can't fill, a frame shorter than the text) reach guards the components keep against states rendering never produces; each says so beside it. The components themselves stay as published.
- **`vitest.config.mts`, not `.ts`**, so it loads as an ES module in a package without `"type": "module"`. Vite warns that it will stop loading a module-syntax `.ts` config from such a package.
