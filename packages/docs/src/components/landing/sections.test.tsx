import type { ReactNode } from 'react';
import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import PixelBlast from '../react-bits/pixel-blast';
import Sections from './sections';
import styles from './sections.module.css';

// Docusaurus provides the colour mode from the site's theme. Each test says
// which mode the visitor is in.
const colorMode = vi.hoisted(() => ({ current: 'light' }));
vi.mock('@docusaurus/theme-common', () => ({
  useColorMode: () => ({ colorMode: colorMode.current }),
}));

// BrowserOnly renders its children only once the page has hydrated, which
// it always has in a browser.
vi.mock('@docusaurus/BrowserOnly', () => ({
  default: ({ children }: { children: () => ReactNode }) => <>{children()}</>,
}));

// Docusaurus's Link needs the site's router. This one is a plain anchor that
// marks a page of the site, which the real one routes to without a reload.
vi.mock('@docusaurus/Link', () => ({
  default: ({ to, href, ...props }: { to?: string; href?: string }) => (
    <a href={to ?? href} data-to={to} {...props} />
  ),
}));

// The closing field needs WebGL 2, which jsdom hasn't got, and has its own
// test. Here it's enough to know what the field asks it to draw.
vi.mock('../react-bits/pixel-blast', () => ({ default: vi.fn(() => null) }));

const REPOSITORY = 'https://github.com/Blotch-Smart-Frames/omni-build';

afterEach(() => {
  colorMode.current = 'light';
  vi.mocked(PixelBlast).mockClear();
});

const windowTitled = (name: string) => screen.getByRole('region', { name });

// Each row as its cells' text, header cell first.
const rows = (table: HTMLElement) =>
  within(table)
    .getAllByRole('row')
    .slice(1)
    .map((row) => [...row.children].map((cell) => cell.textContent));

it('is a window per part of the story, each a section of the outline', () => {
  render(<Sections />);

  expect(
    screen.getAllByRole('heading', { level: 2 }).map((h) => h.textContent),
  ).toEqual([
    'How it works',
    'Before and after',
    'Where it runs',
    'Install',
    'Roadmap',
    'Follow along',
  ]);
  // A window is named by its title, so each is a landmark too.
  expect(windowTitled('Roadmap')).toBeInTheDocument();
});

describe('How it works', () => {
  it('lists where each of the app’s values goes on each platform', () => {
    render(<Sections />);

    const table = within(windowTitled('How it works')).getByRole('table');
    expect(rows(table)).toEqual([
      [
        'App ID',
        'CFBundleIdentifier in Info.plist',
        'package in AndroidManifest.xml',
      ],
      [
        'Name',
        'CFBundleDisplayName in Info.plist',
        'the app label in resources.arsc',
      ],
      [
        'Version',
        'CFBundleShortVersionString, CFBundleVersion',
        'versionName, versionCode',
      ],
      ['Icon', 'Assets.car', 'PNGs per density'],
      ['Payload', 'main.lynx.bundle in the .app', 'assets/main.lynx.bundle'],
      ['Signature', 'Apple code signature', 'APK Signature Scheme v2/v3'],
    ]);
    // On a phone the table stacks, and each cell shows its column's name.
    const [ios, android] = within(table).getAllByRole('cell').slice(0, 2);
    expect(ios).toHaveAttribute('data-label', 'iOS');
    expect(android).toHaveAttribute('data-label', 'Android');
  });

  it('links to the full explanation', () => {
    render(<Sections />);

    expect(
      within(windowTitled('How it works')).getByRole('link', {
        name: 'The full explanation',
      }),
    ).toHaveAttribute('data-to', '/docs/how-it-works');
  });
});

describe('Before and after', () => {
  it('sets what each task needs today against what it will need, the last left as it is', () => {
    render(<Sections />);

    const table = within(windowTitled('Before and after')).getByRole('table');
    expect(rows(table)).toEqual([
      ['Build an Android store binary', 'A JDK and the Android SDK', 'Node'],
      ['Build an iOS store binary', 'A Mac with Xcode', 'Node, on any OS'],
      [
        'Install your build on a phone',
        'Xcode or adb',
        'ADB built in, or TestFlight',
      ],
      [
        'Upload to the stores',
        'Transporter, the Play Console',
        'The stores’ APIs',
      ],
      [
        'Write your own Swift or Kotlin',
        'Xcode or Gradle',
        'Still Xcode or Gradle',
      ],
    ]);
    const body = within(table).getAllByRole('row').slice(1);
    expect(body.map((row) => row.classList.contains(styles.unchanged))).toEqual(
      [false, false, false, false, true],
    );
    expect(
      within(windowTitled('Before and after')).getByText(
        'The right-hand column is the plan, not a release.',
      ),
    ).toBeInTheDocument();
  });
});

describe('Where it runs', () => {
  it('names the six targets, two CPUs per OS', () => {
    render(<Sections />);

    const items = within(windowTitled('Where it runs')).getAllByRole(
      'listitem',
    );
    expect(
      items.map((item) => [...item.children].map((part) => part.textContent)),
    ).toEqual([
      ['macOS', 'Apple silicon and Intel', 'darwin-arm64, darwin-x64'],
      ['Linux', 'ARM64 and x64, static', 'linux-arm64, linux-x64'],
      ['Windows', 'ARM64 and x64', 'win32-arm64, win32-x64'],
    ]);
  });
});

describe('Install', () => {
  it('shows the command dimmed and read-only, because nothing is published yet', () => {
    render(<Sections />);

    const install = windowTitled('Install');
    expect(install).toHaveClass(styles.window, styles.install);
    const field = within(install).getByRole('textbox', {
      name: 'npm, once it’s published',
    });
    expect(field).toHaveAttribute('aria-readonly', 'true');
    expect(field).toHaveAttribute('aria-disabled', 'true');
    expect(field).toHaveTextContent('npm install --save-dev omni-build');
    expect(install).toHaveTextContent('Not published yet.');
    expect(
      within(install).getByRole('link', { name: 'Installation, in full' }),
    ).toHaveAttribute('data-to', '/docs/installation');
  });
});

describe('Roadmap', () => {
  it('ticks what is in place', () => {
    render(<Sections />);

    const [inPlace] = within(windowTitled('Roadmap')).getAllByRole('list');
    const items = within(inPlace).getAllByRole('listitem');
    expect(items.map((item) => item.textContent)).toEqual([
      'Release builds for all six targets, each run on its own OS and CPU',
      '100% test coverage of the Rust and the TypeScript',
      'End-to-end runs on the iOS Simulator and an Android emulator',
    ]);
    items.forEach((item) => {
      expect(item.querySelector(`.${styles.checkbox} svg`)).not.toBeNull();
    });
  });

  it('numbers the phases from 0, none ticked, and marks the first as next', () => {
    render(<Sections />);

    const [, phases] = within(windowTitled('Roadmap')).getAllByRole('list');
    expect(phases).toHaveAttribute('start', '0');
    const items = within(phases).getAllByRole('listitem');
    // Each phase's number and name, without the tag.
    const phase = (item: HTMLElement) =>
      [...item.querySelector(`.${styles.phase}`)!.childNodes]
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent)
        .join('');
    expect(items.map(phase)).toEqual([
      '0. Spikes',
      '1. Dev client',
      '2. Android',
      '3. iOS',
      '4. ng deploy',
      '5. Native modules',
    ]);
    expect(
      items.map(
        (item) => item.querySelector(`.${styles.nextTag}`)?.textContent,
      ),
    ).toEqual(['next', undefined, undefined, undefined, undefined, undefined]);
    expect(items.map((item) => item.classList.contains(styles.next))).toEqual([
      true,
      false,
      false,
      false,
      false,
      false,
    ]);
    items.forEach((item) => {
      expect(item.querySelector(`.${styles.checkbox}`)).toBeEmptyDOMElement();
    });
    expect(items[0].querySelector(`.${styles.scope}`)).toHaveTextContent(
      'Rewrite and sign a prebuilt shell on each platform; go or no-go',
    );
  });

  it('links to the phases, risks and open decisions', () => {
    render(<Sections />);

    expect(
      within(windowTitled('Roadmap')).getByRole('link', {
        name: 'Phases, risks and open decisions',
      }),
    ).toHaveAttribute('data-to', '/docs/roadmap');
  });
});

describe('Follow along', () => {
  it('invites the reader to the plan and the repository', () => {
    render(<Sections />);

    const follow = windowTitled('Follow along');
    expect(
      within(follow).getByRole('link', { name: 'Read the plan' }),
    ).toHaveAttribute('href', `${REPOSITORY}/issues/1`);
    expect(
      within(follow).getByRole('link', { name: 'Watch on GitHub' }),
    ).toHaveAttribute('href', REPOSITORY);
  });

  it('closes on a field of hard one-bit pixels in ink on paper', () => {
    render(<Sections />);

    expect(vi.mocked(PixelBlast).mock.lastCall![0]).toEqual({
      variant: 'square',
      pixelSize: 4,
      color: '#000000',
      accentColor: '#000000',
      lightMode: true,
      transparent: false,
      patternScale: 2.5,
      patternDensity: 0.95,
      softness: 0,
      glow: 0,
      twinkle: 0,
      edgeFade: 0,
      pixelSizeJitter: 0,
      noiseAmount: 0,
      speed: 0.35,
      hoverEffect: 'reveal',
      hoverRadius: 120,
      enableRipples: true,
    });
    expect(document.querySelector(`.${styles.blast}`)).toHaveAttribute(
      'aria-hidden',
      'true',
    );
  });

  it('draws the field in paper on ink in dark mode', () => {
    colorMode.current = 'dark';

    render(<Sections />);

    expect(vi.mocked(PixelBlast).mock.lastCall![0]).toMatchObject({
      color: '#ffffff',
      accentColor: '#ffffff',
      lightMode: false,
    });
  });
});
