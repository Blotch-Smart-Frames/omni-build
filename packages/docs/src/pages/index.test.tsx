import type { ReactNode } from 'react';
import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';

import Home from './index';
import styles from './index.module.css';

// The theme's Layout needs the whole Docusaurus site around it (navbar,
// footer, router). This one shows what the page gives it.
vi.mock('@theme/Layout', () => ({
  default: ({
    title,
    description,
    children,
  }: {
    title: string;
    description: string;
    children: ReactNode;
  }) => (
    <div data-testid="layout" data-title={title} data-description={description}>
      {children}
    </div>
  ),
}));

// The parts of the page have tests of their own; here they only need to be
// told apart.
vi.mock('../components/landing/desktop-field', () => ({
  default: () => <div data-testid="field" />,
}));
vi.mock('../components/landing/hero', () => ({
  default: () => <div data-testid="hero" />,
}));
vi.mock('../components/landing/sections', () => ({
  default: () => <div data-testid="sections" />,
}));

it('titles and describes the page for search engines and link previews', () => {
  render(<Home />);

  const layout = screen.getByTestId('layout');
  expect(layout).toHaveAttribute(
    'data-title',
    'iOS and Android apps with no native toolchain',
  );
  // The description says it's in planning, as the page does.
  expect(layout.dataset.description).toMatch(
    /^OmniBuild builds, signs, installs and ships iOS and Android apps .* In planning\.$/,
  );
});

it('lays the hero and the sections over the dithered desktop, in that order', () => {
  render(<Home />);

  const main = screen.getByRole('main');
  expect(main).toHaveClass(styles.page, { exact: true });
  expect(
    [...main.children].map((child) => child.getAttribute('data-testid')),
  ).toEqual(['field', 'hero', 'sections']);
});
