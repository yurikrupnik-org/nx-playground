// Belt and braces for the Postgres container: Playwright kills the
// `docker run` client in its webServer teardown, but a SIGKILL'd client does
// not stop the container it was attached to. Remove it by name here so an
// aborted run never leaves a database bound to the e2e port.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import { STACK } from './stack';

export default async function teardown() {
  await promisify(execFile)('docker', ['rm', '-f', STACK.container]).catch(
    () => undefined,
  );
}
