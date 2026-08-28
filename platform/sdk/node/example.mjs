/**
 * Print DevEnvironment status and, when ready, its connection details.
 *
 * Usage: bun example.mjs [claim-name] [namespace]
 */
import { get, connection } from './devenv.mjs';

const name = process.argv[2] ?? 'demo';
const namespace = process.argv[3] ?? 'default';

const status = await get(name, namespace);
console.log(JSON.stringify(status, null, 2));

if (status.ready) {
  const conn = await connection(name, namespace);
  if (conn.postgres?.password) {
    conn.postgres.password = '********';
    if (conn.postgres.uri) conn.postgres.uri = conn.postgres.uri.replace(/\/\/([^:]+):[^@]+@/, '//$1:********@');
  }
  console.log(JSON.stringify(conn, null, 2));
} else {
  console.error(`DevEnvironment ${namespace}/${name} is not ready yet`);
}
