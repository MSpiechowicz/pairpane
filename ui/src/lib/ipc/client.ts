import { createTauriTransport } from './tauri-transport';
import { IpcFailure, type Transport } from './transport';

/** Used when the Tauri runtime is missing (e.g. plain `vite` in a browser). */
function createUnavailableTransport(): Transport {
  const failure = new IpcFailure({
    code: 'internal',
    message: 'The desktop runtime is not available.',
  });

  return {
    invoke: () => Promise.reject(failure),
  };
}

/** Picks the transport for this environment. */
export function selectTransport(hasTauri: boolean = '__TAURI_INTERNALS__' in window): Transport {
  if (hasTauri) {
    return createTauriTransport();
  }

  return createUnavailableTransport();
}

let selected: Transport | undefined;

function getTransport(): Transport {
  selected ??= selectTransport();

  return selected;
}

/** The transport every feature talks to; resolves the real one lazily on first use. */
export const ipc: Transport = {
  invoke(name, args) {
    return getTransport().invoke(name, args);
  },
};
