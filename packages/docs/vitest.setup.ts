import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// Matchers for the DOM (toHaveAttribute, toBeVisible, ...), typed for
// vitest's expect.
import '@testing-library/jest-dom/vitest';

// React Testing Library unmounts what each test rendered by itself only
// when the test runner's globals are on, and they're off here.
afterEach(cleanup);
