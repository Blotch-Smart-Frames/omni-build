# @omni-build/docs

OmniBuild's documentation site and landing page, built with [Docusaurus](https://docusaurus.io) and published to GitHub Pages at https://blotch-smart-frames.github.io/omni-build/.

```sh
npm start -w @omni-build/docs        # dev server with live reload
npm run build -w @omni-build/docs    # the production site, in build/
npm run serve -w @omni-build/docs    # serve build/ locally
```

The pages are in `docs/`. The landing page is `src/pages/index.jsx`, and its components are in `src/components/`.

## Publishing

`.github/workflows/docs.yml` builds and deploys the site from `master`. The CI gate's `docs` job builds it on every pull request, and a broken link or anchor fails the build.

## Things to know

- **No `"type": "module"`** in `package.json`. Docusaurus generates a route registry that uses webpack's `require.resolveWeak`, and the bundler only handles that in files it treats as CommonJS. The config files still use `import` and `export`: Docusaurus loads them either way.
- **The landing page's animations come from [React Bits](https://reactbits.dev)**, copied in as source, which is how React Bits is distributed. They're in `src/components/react-bits/`, renamed to kebab-case for the repository's lint rules, with React Bits' licence beside them. Its terms allow them in a website, but not redistributing the components themselves.
- **No unit tests or coverage gate.** The site is content and presentation; the build, with its broken-link checks, is its test.
