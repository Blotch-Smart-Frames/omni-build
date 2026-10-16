import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

/** How much of a failed command's output an error shows. The rest is in the log. */
const TAIL_LINES = 40;

const tail = (output) =>
  output.trimEnd().split('\n').slice(-TAIL_LINES).join('\n');

/**
 * Runs a command to the end, appending the command and its output to `log`.
 * Throws with the end of the output when it fails, unless `allowFailure`.
 *
 * @param {string} command
 * @param {string[]} args
 * @param {object} options
 * @param {string} [options.log] Left out for commands run in a polling loop,
 *   which would bury everything else in the log.
 * @param {string} [options.cwd]
 * @param {NodeJS.ProcessEnv} [options.env]
 * @param {boolean} [options.allowFailure]
 * @returns {Promise<{ code: number, stdout: string, output: string }>}
 *   `output` is stdout and stderr interleaved, as a terminal shows them.
 */
export const run = (command, args, { log, cwd, env, allowFailure = false }) =>
  new Promise((resolve, reject) => {
    const record = (text) => log && appendFileSync(log, text);
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
 *
 * @template T
 * @param {() => Promise<T | undefined>} attempt
 * @param {object} options
 * @param {number} options.timeoutMs
 * @param {string} options.message
 * @returns {Promise<T>}
 */
export const poll = async (attempt, { timeoutMs, message }) => {
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
export const step = (message) => console.log(`\n▸ ${message}`);
