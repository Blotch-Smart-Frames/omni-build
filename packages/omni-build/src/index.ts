import { spawn } from 'node:child_process';

import { resolveBinary } from './resolve-binary.ts';

// The package's API, for build tools: omniBuild() runs the binary with
// --json, as esbuild's package runs esbuild, and hands the tool each event
// the binary reports, typed. The events mirror packages/cli/src/report.rs.
// Every event has a message, the sentence a person would read, so a tool
// can show events a newer binary adds before it knows them.

export { resolveBinary };

/** A key OmniBuild created. */
export interface KeyCreatedEvent {
  event: 'key-created';
  /**
   * What it's for: `debug` signs Android apps when OmniBuild is given no key,
   * `adb` proves this computer's identity to Android devices, and `upload`,
   * from `key create`, signs app bundles for Google Play.
   */
  key: 'debug' | 'adb' | 'upload';
  path: string;
  message: string;
}

/**
 * The device asks its user whether to allow this computer, and OmniBuild
 * waits for the answer: a tool should show the message.
 */
export interface AuthorizingEvent {
  event: 'authorizing';
  device: string;
  message: string;
}

/** `repackage` wrote the app. */
export interface RepackagedEvent {
  event: 'repackaged';
  shell: string;
  out: string;
  platform: 'android' | 'ios';
  /** `app` is a zipped `.app`, for the iOS Simulator. */
  format: 'apk' | 'aab' | 'ipa' | 'app';
  message: string;
}

/** `install` installed the app. */
export interface InstalledEvent {
  event: 'installed';
  app: string;
  /** The device's name, as `--device` takes it. */
  device: string;
  /** The device's model, as its user knows it, if it says. */
  model?: string;
  message: string;
}

/** Why the command failed: the last event. */
export interface ErrorEvent {
  event: 'error';
  message: string;
}

export type OmniBuildEvent =
  | KeyCreatedEvent
  | AuthorizingEvent
  | RepackagedEvent
  | InstalledEvent
  | ErrorEvent;

/** A command that failed, with omni-build's reason as its message. */
export class OmniBuildError extends Error {
  name = 'OmniBuildError';
}

export interface OmniBuildOptions {
  /** Called with each event as omni-build reports it, the last included. */
  onEvent?: (event: OmniBuildEvent) => void;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  /** The binary to run, rather than the one resolveBinary() finds. */
  binary?: string;
}

/**
 * Runs an omni-build command, such as `['repackage', 'shell.apk', '--out',
 * 'app.apk']`, and resolves with the last event it reports: its result, such
 * as a `repackaged` event. Rejects with an OmniBuildError when it fails.
 */
export const omniBuild = (
  args: string[],
  { onEvent, cwd, env, binary }: OmniBuildOptions = {},
) =>
  new Promise<OmniBuildEvent>((resolve, reject) => {
    let command;
    try {
      command = binary ?? resolveBinary();
    } catch (error) {
      reject(new OmniBuildError((error as Error).message));
      return;
    }

    const child = spawn(command, [...args, '--json'], {
      cwd,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let pending = '';
    let stderr = '';
    let last: OmniBuildEvent | undefined;
    let failure: unknown;

    const take = (line: string) => {
      try {
        last = JSON.parse(line) as OmniBuildEvent;
      } catch {
        throw new OmniBuildError(
          `omni-build wrote a line that isn't JSON: ${line}`,
        );
      }
      onEvent?.(last);
    };

    // A chunk can end partway through a line, or hold several.
    child.stdout.setEncoding('utf8').on('data', (chunk: string) => {
      pending += chunk;
      let end;
      while (failure === undefined && (end = pending.indexOf('\n')) !== -1) {
        const line = pending.slice(0, end);
        pending = pending.slice(end + 1);
        try {
          take(line);
        } catch (error) {
          // Stop the command: the tool can't follow it any more.
          failure = error;
          child.kill();
        }
      }
    });
    // Only for what omni-build says before it reads --json, about its
    // arguments, and for a crash.
    child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
      stderr += chunk;
    });
    child.on('error', (error) =>
      reject(new OmniBuildError(`couldn't run ${command}: ${error.message}`)),
    );
    child.on('close', (code, signal) => {
      if (failure !== undefined) {
        reject(failure);
      } else if (last?.event === 'error') {
        reject(new OmniBuildError(last.message));
      } else if (code === 0 && last) {
        resolve(last);
      } else {
        const ended = signal
          ? `was stopped by ${signal}`
          : code === 0
            ? 'reported nothing'
            : `exited with ${code}`;
        const said = stderr.trim();
        reject(new OmniBuildError(`omni-build ${ended}${said && `: ${said}`}`));
      }
    });
  });
