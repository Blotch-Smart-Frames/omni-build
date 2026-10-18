import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';

import Button from './button';
import styles from './button.module.css';

// Docusaurus's Link needs the site's router. This one is a plain anchor that
// marks a page of the site, which the real one routes to without a reload.
vi.mock('@docusaurus/Link', () => ({
  default: ({ to, href, ...props }: { to?: string; href?: string }) => (
    <a href={to ?? href} data-to={to} {...props} />
  ),
}));

it('links to a page of the site', () => {
  render(<Button to="/docs">Read the docs</Button>);

  const link = screen.getByRole('link', { name: 'Read the docs' });
  expect(link).toHaveAttribute('data-to', '/docs');
  expect(link).toHaveClass(styles.button, { exact: true });
});

it('links to an address elsewhere, as the default button', () => {
  render(
    <Button href="https://example.com" primary className="extra" title="Out">
      Watch
    </Button>,
  );

  const link = screen.getByRole('link', { name: 'Watch' });
  expect(link).toHaveAttribute('href', 'https://example.com');
  expect(link).not.toHaveAttribute('data-to');
  expect(link).toHaveAttribute('title', 'Out');
  expect(link).toHaveClass(`${styles.button} ${styles.primary} extra`, {
    exact: true,
  });
});

it('is a button, and never a submit button, when it goes nowhere', async () => {
  const onClick = vi.fn();
  render(
    <Button onClick={onClick} className="extra">
      Run again
    </Button>,
  );

  const button = screen.getByRole('button', { name: 'Run again' });
  expect(button).toHaveAttribute('type', 'button');
  expect(button).toHaveClass(`${styles.button} extra`, { exact: true });

  await userEvent.click(button);

  expect(onClick).toHaveBeenCalledOnce();
});
