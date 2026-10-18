import type * as Preset from '@docusaurus/preset-classic';
import type { ThemeConfig } from '@docusaurus/theme-common';
import type { Config } from '@docusaurus/types';

const repository = 'https://github.com/Blotch-Smart-Frames/omni-build';

// Syntax highlighting in one bit: no colours to tell tokens apart, so
// keywords are bold and comments italic, as on a one-bit screen. The colours
// are the theme's variables, so the same theme serves dark mode.
const oneBitCode: ThemeConfig['prism']['theme'] = {
  plain: { color: 'var(--ink)', backgroundColor: 'var(--paper)' },
  styles: [
    {
      types: ['comment', 'prolog', 'doctype', 'cdata'],
      style: { fontStyle: 'italic' },
    },
    {
      types: ['keyword', 'builtin', 'tag', 'selector', 'atrule', 'important'],
      style: { fontWeight: 'bold' },
    },
    { types: ['deleted'], style: { textDecorationLine: 'line-through' } },
  ],
};

const config: Config = {
  title: 'OmniBuild',
  tagline:
    'Build, sign, install and ship iOS and Android apps with no native toolchain',
  favicon: 'img/favicon.svg',

  future: {
    // Opts in to v4's defaults now, so the upgrade is a version bump. It also
    // turns on @docusaurus/faster: Rspack and SWC instead of webpack and
    // Babel.
    v4: true,
  },

  // GitHub Pages serves a project site from /<repository>/. `trailingSlash`
  // must be set for GitHub Pages, which otherwise redirects /docs/roadmap to
  // /docs/roadmap/ and loses the anchor in links like /docs/roadmap#risks.
  url: 'https://blotch-smart-frames.github.io',
  baseUrl: '/omni-build/',
  trailingSlash: false,
  organizationName: 'Blotch-Smart-Frames',
  projectName: 'omni-build',

  // A broken link or anchor fails the build, so CI catches it before the
  // site does.
  onBrokenLinks: 'throw',
  onBrokenAnchors: 'throw',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  clientModules: ['./src/client-modules/fonts.ts'],

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl: `${repository}/edit/master/packages/docs/`,
        },
        // No blog until there's something to announce.
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'OmniBuild',
      logo: {
        alt: '',
        src: 'img/logo.svg',
        srcDark: 'img/logo-dark.svg',
        width: 32,
        height: 32,
      },
      items: [
        {
          // A menu, as the one-bit desktop's menu bar had: every page,
          // one click from anywhere.
          type: 'dropdown',
          label: 'Docs',
          position: 'left',
          items: [
            { type: 'doc', docId: 'intro', label: 'Introduction' },
            { type: 'doc', docId: 'how-it-works', label: 'How it works' },
            { type: 'doc', docId: 'installation', label: 'Installation' },
            { type: 'doc', docId: 'angularlynx', label: 'AngularLynx' },
            { type: 'doc', docId: 'roadmap', label: 'Roadmap' },
            { type: 'doc', docId: 'contributing', label: 'Contributing' },
          ],
        },
        {
          href: `${repository}/issues/1`,
          label: 'The plan',
          position: 'left',
        },
        {
          type: 'html',
          position: 'right',
          value:
            '<span class="menubar-status">Planning: nothing to install yet</span>',
        },
        {
          href: repository,
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      links: [
        {
          title: 'Docs',
          items: [
            { label: 'Introduction', to: '/docs' },
            { label: 'How it works', to: '/docs/how-it-works' },
            { label: 'Roadmap', to: '/docs/roadmap' },
          ],
        },
        {
          title: 'Project',
          items: [
            { label: 'GitHub', href: repository },
            { label: 'The plan', href: `${repository}/issues/1` },
            {
              label: 'AngularLynx',
              href: 'https://github.com/Blotch-Smart-Frames/AngularLynx',
            },
          ],
        },
      ],
      copyright: `MIT licensed. Built with Docusaurus.`,
    },
    prism: {
      theme: oneBitCode,
      darkTheme: oneBitCode,
      additionalLanguages: ['bash', 'json', 'rust'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
