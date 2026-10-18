import { execFile } from 'node:child_process';

export interface ExecResult {
  ok: boolean;
  /** What the command printed: stdout, then stderr. */
  output: string;
}

export type Exec = (
  command: string,
  args: string[],
  cwd: string,
) => Promise<ExecResult>;

/**
 * Runs a command to completion. A failure, a command that isn't installed
 * included, is a result rather than an exception: the checks report it, with
 * what the command printed, as one problem among the others.
 */
export const exec: Exec = (command, args, cwd) =>
  new Promise((resolve) => {
    execFile(
      command,
      args,
      // A failing test run prints a lot; past execFile's 1 MB default it
      // would be killed and its output lost.
      { cwd, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
      (error, stdout, stderr) => {
        resolve({
          ok: !error,
          // A command that never started printed nothing; the error says why.
          output: `${stdout}${stderr}` || (error?.message ?? ''),
        });
      },
    );
  });
