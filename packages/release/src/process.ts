import { spawn } from 'node:child_process';
import path from 'node:path';

export interface ExecOptions {
  cwd?: string;
  env?: NodeJS.ProcessEnv;
}

/** Runs a command and returns its output, as it ran it. */
export type Exec = (
  command: string,
  args: string[],
  options?: ExecOptions,
) => Promise<string>;

/**
 * Runs a command to the end and returns its stdout. Throws with everything
 * it printed when it fails.
 */
export const exec: Exec = (command, args, { cwd, env } = {}) =>
  new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let output = '';
    child.stdout.setEncoding('utf8').on('data', (chunk: string) => {
      stdout += chunk;
      output += chunk;
    });
    child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
      output += chunk;
    });
    child.on('error', (error) =>
      reject(new Error(`couldn't run ${command}: ${error.message}`)),
    );
    child.on('close', (code, signal) => {
      if (code === 0) {
        resolve(stdout);
      } else {
        reject(
          new Error(
            `\`${[command, ...args].join(' ')}\` failed (${signal ?? `exit ${code}`}):\n${output.trimEnd()}`,
          ),
        );
      }
    });
  });

/**
 * The command that runs npm: Node, with npm's own script. On Windows `npm`
 * is a batch file, which Node only starts through a shell, so npm's script
 * is run directly everywhere: the one `npm run` names in `npm_execpath`, or
 * the one installed beside Node.
 */
export const npmCommand = ({
  env = process.env,
  execPath = process.execPath,
  platform = process.platform,
}: {
  env?: NodeJS.ProcessEnv;
  execPath?: string;
  platform?: string;
} = {}): [string, string] => {
  if (env.npm_execpath) {
    return [execPath, env.npm_execpath];
  }
  // Node's installers put npm in lib/node_modules beside bin/node, and on
  // Windows in node_modules beside node.exe.
  const prefix =
    platform === 'win32'
      ? path.dirname(execPath)
      : path.join(path.dirname(execPath), '..', 'lib');
  return [
    execPath,
    path.join(prefix, 'node_modules', 'npm', 'bin', 'npm-cli.js'),
  ];
};
