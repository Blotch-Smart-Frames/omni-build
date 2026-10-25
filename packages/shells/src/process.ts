import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';

/** How much of a failed command's output an error shows. The rest is in the log. */
const TAIL_LINES = 40;

const tail = (output: string) =>
  output.trimEnd().split('\n').slice(-TAIL_LINES).join('\n');

export interface RunOptions {
  /**
   * A file to append the command and its output to. Without one, they go to
   * stdout, as a CI job's log shows them.
   */
  log?: string;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  allowFailure?: boolean;
}

export interface RunResult {
  code: number;
  stdout: string;
  /** stdout and stderr interleaved, as a terminal shows them. */
  output: string;
}

/**
 * Runs a command to the end, recording the command and its output (see
 * RunOptions.log). Throws with the end of the output when it fails, unless
 * `allowFailure`.
 */
export const run = (
  command: string,
  args: string[],
  { log, cwd, env, allowFailure = false }: RunOptions = {},
) =>
  new Promise<RunResult>((resolve, reject) => {
    const record = (text: string) =>
      log ? appendFileSync(log, text) : process.stdout.write(text);
    record(`\n$ ${[command, ...args].join(' ')}\n`);
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
      record(chunk);
    });
    child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
      output += chunk;
      record(chunk);
    });
    child.on('error', (error) =>
      reject(new Error(`couldn't run ${command}: ${error.message}`)),
    );
    child.on('close', (code, signal) => {
      if (code !== 0 && !allowFailure) {
        reject(
          new Error(
            `\`${[command, ...args].join(' ')}\` failed (${signal ?? `exit ${code}`}):\n${tail(output)}`,
          ),
        );
        return;
      }
      resolve({ code: code ?? 1, stdout, output });
    });
  });
