import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';

import { DEFAULT_PREFERENCES, preferences } from '../src/lib/shell/preferences.svelte';

// The preferences are the app's one: what a test sets does not reach the next.
afterEach(() => preferences.restore({ ...DEFAULT_PREFERENCES }));
