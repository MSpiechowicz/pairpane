import { readFileSync } from 'node:fs';

/**
 * The app's version, from the repository's package.json (semantic-release bumps it together with Cargo.toml).
 * The build and the tests inject it as `__APP_VERSION__`.
 */
export const appVersion: string = JSON.parse(
  readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
).version;
