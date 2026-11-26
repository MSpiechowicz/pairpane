import type { ConnectTarget } from '$lib/ipc/bindings/ConnectTarget';

/** What the Connect dialog's fields hold, as typed. */
export interface ConnectForm {
  host: string;
  user: string;
  port: string;
  keyFile: string;
}

/** The lowest and the highest port a server can listen on. */
const PORTS = { min: 1, max: 65_535 };

/** What the fields ask for, or why they cannot be sent. A field left empty is the configuration's (or the default). */
export function toTarget(form: ConnectForm): ConnectTarget | string {
  const host = form.host.trim();
  if (host === '') {
    return 'Enter the host to connect to.';
  }

  const typed = form.port.trim();
  const port = typed === '' ? null : Number(typed);
  if (port !== null && !(/^\d+$/.test(typed) && port >= PORTS.min && port <= PORTS.max)) {
    return `The port is a number from ${PORTS.min} to ${PORTS.max}.`;
  }

  return { host, port, user: form.user.trim() || null, keyFile: form.keyFile.trim() || null };
}
