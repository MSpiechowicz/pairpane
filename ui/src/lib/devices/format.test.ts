import { describe, expect, it } from 'vitest';

import { countdown, keyStorageNote, pairingEndText, shortId, statusText } from './format';

describe('shortId', () => {
  it('keeps the first six and the last four characters of a long id', () => {
    expect(shortId('a1b2c3d4e5f6a7b8c9d0')).toBe('a1b2c3…c9d0');
  });

  it('leaves a short one as it is', () => {
    expect(shortId('abc123')).toBe('abc123');
  });
});

describe('countdown', () => {
  it.each([
    [90_000, '1:30'],
    [59_001, '1:00'],
    [5_000, '0:05'],
    [1, '0:01'],
    [0, '0:00'],
    [-40, '0:00'],
  ])('%d ms is %s', (millis, text) => {
    expect(countdown(millis)).toBe(text);
  });
});

describe('keyStorageNote', () => {
  it('says why a file holds the key when there is no keychain', () => {
    expect(keyStorageNote('file')).toBe(
      "This system has no keychain (Secret Service), so the device key is kept in a private file (readable only by you) in Pairpane's data folder.",
    );
  });

  it('tells each place apart', () => {
    expect(keyStorageNote('keychain')).toMatch(/keychain/);
    expect(keyStorageNote('memory')).toMatch(/memory only/);
  });
});

describe('statusText', () => {
  it('names the device only when it calls itself something else', () => {
    expect(statusText({ kind: 'online', name: 'Laptop' }, 'Laptop')).toBe('Online');
    expect(statusText({ kind: 'online', name: 'Mac' }, 'Laptop')).toBe('Online as Mac');
  });

  it('says what each other status is', () => {
    expect(statusText('checking', 'Laptop')).toBe('Checking…');
    expect(statusText({ kind: 'offline' }, 'Laptop')).toBe('Offline');
    expect(statusText({ kind: 'refused' }, 'Laptop')).toMatch(/^Refused/);
  });
});

describe('pairingEndText', () => {
  it('gives every reason its own words', () => {
    const texts = (['used', 'expired', 'cancelled', 'tooManyAttempts'] as const).map(pairingEndText);

    expect(new Set(texts).size).toBe(4);
    expect(pairingEndText('tooManyAttempts')).toMatch(/Too many/);
  });
});
