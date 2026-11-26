import { describe, expect, it } from 'vitest';

import { toTarget } from './target';

const form = (extra: Partial<Parameters<typeof toTarget>[0]> = {}) => ({ host: 'files.example.com', user: '', port: '', keyFile: '', ...extra });

describe('toTarget', () => {
  it('sends only the host when the other fields are empty: the configuration decides the rest', () => {
    expect(toTarget(form())).toEqual({ host: 'files.example.com', port: null, user: null, keyFile: null });
  });

  it('trims what was typed and reads the port as a number', () => {
    expect(toTarget(form({ host: ' work ', user: ' ann ', port: ' 2222 ', keyFile: ' ~/.ssh/work ' }))).toEqual({
      host: 'work',
      port: 2222,
      user: 'ann',
      keyFile: '~/.ssh/work',
    });
  });

  it.each(['', '   '])('asks for a host (%j)', (host) => {
    expect(toTarget(form({ host }))).toBe('Enter the host to connect to.');
  });

  it.each(['0', '65536', '22a', '-1', '2.5', '1e3'])('refuses the port %j', (port) => {
    expect(toTarget(form({ port }))).toBe('The port is a number from 1 to 65535.');
  });

  it('takes the ends of the range', () => {
    expect(toTarget(form({ port: '1' }))).toMatchObject({ port: 1 });
    expect(toTarget(form({ port: '65535' }))).toMatchObject({ port: 65535 });
  });
});
