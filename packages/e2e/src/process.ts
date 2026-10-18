import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

/** How much of a failed command's output an error shows. The rest is in the log. */
const TAIL_LINES = 40;

const tail = (output: string) =>
  output.trimEnd().split('\n').slice(-TAIL_LINES).join('\n');

export interface RunOptions {
  /**
   * Left out for commands run in a polling loop, which would bury everything
   * else in the log.
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
 * Runs a command to the end, appending the command and its output to `log`.
 * Throws with the end of the output when it fails, unless `allowFailure`.
 */
export const run = (
  command: string,
  args: string[],
  { log, cwd, env, allowFailure = false }: RunOptions,
) =>
  new Promise<RunResult>((resolve, reject) => {
    const record = (text: string) => log && appendFileSync(log, text);
    record(`\n$ ${[command, ...args].join(' ')}\n`);
    const child = spawn(command, args, {
      cwd,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let output = '';
    child.stdout.setEncoding('utf8').on('data', (chunk) => {
      stdout += chunk;
      output += chunk;
      record(chunk);
    });
    child.stderr.setEncoding('utf8').on('data', (chunk) => {
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

/**
 * Calls `attempt` until it returns something other than undefined, and
 * returns that. Throws `message` after `timeoutMs`.
 */
export const poll = async <T>(
  attempt: () => Promise<T | undefined>,
  { timeoutMs, message }: { timeoutMs: number; message: string },
): Promise<T> => {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const result = await attempt();
    if (result !== undefined) {
      return result;
    }
    if (Date.now() > deadline) {
      throw new Error(message);
    }
    await sleep(500);
  }
};

/** Prints the start of one step of the run. */
export const step = (message: string) => console.log(`\n▸ ${message}`);
