import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';

import DesktopIcon from './desktop-icon';
import styles from './desktop-icon.module.css';

// Docusaurus's Link needs the site's router. This one is a plain anchor that
// marks a page of the site, which the real one routes to without a reload.
vi.mock('@docusaurus/Link', () => ({
  default: ({ to, href, ...props }: { to?: string; href?: string }) => (
    <a href={to ?? href} data-to={to} {...props} />
  ),
}));

it('opens a page of the site', () => {
  const ref = createRef<HTMLElement>();
  render(<DesktopIcon ref={ref} icon="document" label="Read me" to="/docs" />);

  const link = screen.getByRole('link', { name: 'Read me' });
  expect(link).toHaveAttribute('data-to', '/docs');
  expect(link).toHaveClass(styles.icon, { exact: true });
  expect(ref.current).toBe(link);
});

it('opens an address elsewhere', () => {
  render(
    <DesktopIcon
      icon="document"
      label="The plan"
      href="https://example.com/plan"
      selected
    />,
  );

  const link = screen.getByRole('link', { name: 'The plan' });
  expect(link).toHaveAttribute('href', 'https://example.com/plan');
  expect(link).toHaveClass(`${styles.icon} ${styles.selected}`, {
    exact: true,
  });
});

it('is a button when it acts', async () => {
  const onClick = vi.fn();
  const ref = createRef<HTMLElement>();
  render(
    <DesktopIcon
      ref={ref}
      icon="omnibuild"
      label="OmniBuild"
      onClick={onClick}
      aria-label="OmniBuild: run the demonstration"
      className="extra"
    />,
  );

  const button = screen.getByRole('button', {
    name: 'OmniBuild: run the demonstration',
  });
  expect(button).toHaveAttribute('type', 'button');
  expect(button).toHaveClass(`${styles.icon} extra`, { exact: true });
  expect(ref.current).toBe(button);

  await userEvent.click(button);

  expect(onClick).toHaveBeenCalledOnce();
});

it('is a plain figure when it only shows a file', () => {
  const ref = createRef<HTMLElement>();
  const { container } = render(
    <DesktopIcon ref={ref} icon="package" label="Tides.apk" selected />,
  );

  const icon = container.firstElementChild!;
  expect(icon.tagName).toBe('DIV');
  expect(icon).toHaveClass(`${styles.icon} ${styles.selected}`, {
    exact: true,
  });
  expect(ref.current).toBe(icon);
  expect(screen.queryByRole('link')).toBeNull();
  expect(screen.queryByRole('button')).toBeNull();
});

it('shows its glyph at three times size over its label', () => {
  const { container } = render(<DesktopIcon icon="folder" label="Tides" />);

  const glyph = container.querySelector('svg')!;
  expect(glyph).toHaveClass(styles.glyph);
  // 16 pixels a side, three times over.
  expect(glyph).toHaveAttribute('width', '48');
  expect(glyph).toHaveAttribute('aria-hidden', 'true');
  expect(container.querySelector(`.${styles.label}`)).toHaveTextContent(
    'Tides',
  );
});
