import { setTimeout as sleep } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import { findings } from './findings.ts';
import { apply, gitHub, plan, summarize } from './issues.ts';
import { type Fetch, type Wait, readSources } from './read.ts';
import { SOURCES, type SourceReadings } from './sources.ts';

const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url));

export const HELP = `Usage: npm run check -w @blotch/omni-build-platform-watch -- [options]

Compares the toolchain the app shells are built with against what Apple and
Google require and have released, and opens, updates and closes GitHub
issues to match. A source it can't read or parse gets an issue too. CI runs
it daily (.github/workflows/platform-watch.yml).

Options:
  --repo <owner/name>  The repository the issues are in.
                       Default: $GITHUB_REPOSITORY.
  --dry-run            Print what would change on GitHub, and change nothing.
  --help               Show this.

Environment:
  GITHUB_TOKEN  A token that can write issues. --dry-run reads a public
                repository's issues without one.`;

export interface RunOptions {
  /** The repository root. */
  root?: string;
  env?: Record<string, string | undefined>;
  fetch?: Fetch;
  wait?: Wait;
  log?: (line: string) => void;
}

export interface RunResult {
  status: number;
  /** For stderr. */
  message?: string;
}

/**
 * Runs one check: reads every source, works out the issues there should be,
 * and makes GitHub's match.
 *
 * The options default to the real repository, network and console, and
 * exist for tests.
 */
export const run = async (
  args: string[],
  {
    root = REPO_ROOT,
    env = process.env,
    fetch = globalThis.fetch,
    wait = sleep,
    log = console.log,
  }: RunOptions = {},
): Promise<RunResult> => {
  let values;
  try {
    ({ values } = parseArgs({
      args,
      options: {
        repo: { type: 'string' },
        'dry-run': { type: 'boolean', default: false },
        help: { type: 'boolean', default: false },
      },
    }));
  } catch (error) {
    return {
      status: 2,
      message: `platform-watch: ${(error as Error).message}`,
    };
  }
  if (values.help) {
    log(HELP);
    return { status: 0 };
  }
  const repo = values.repo ?? env.GITHUB_REPOSITORY;
  if (!repo) {
    return {
      status: 2,
      message:
        'platform-watch: no repository: pass --repo <owner/name> or set GITHUB_REPOSITORY',
    };
  }
  const dryRun = values['dry-run'];
  if (!dryRun && !env.GITHUB_TOKEN) {
    return {
      status: 2,
      message:
        'platform-watch: changing issues needs GITHUB_TOKEN. --dry-run only prints what would change',
    };
  }

  // readSources() reads every source it's given, so each of SOURCES has a
  // reading.
  const readings = (await readSources(SOURCES, {
    root,
    fetch,
    wait,
  })) as SourceReadings;
  for (const [id, reading] of readings) {
    log(`${id}: ${reading.ok ? JSON.stringify(reading.value) : reading.error}`);
  }

  // A source the watch couldn't read isn't a failed run: it's reported as
  // an issue like any other finding. The run fails only when it can't
  // report, because GitHub refused or was out of reach.
  const github = gitHub({ repo, token: env.GITHUB_TOKEN, fetch });
  try {
    const actions = await plan(findings(readings), readings, github);
    if (actions.length === 0) {
      log('Nothing to change on GitHub.');
    } else {
      log(dryRun ? 'Would change on GitHub:' : 'Changing on GitHub:');
    }
    for (const action of actions) {
      if (!dryRun) {
        await apply(action, github);
      }
      log(`- ${summarize(action)}`);
    }
  } catch (error) {
    return {
      status: 1,
      message: `platform-watch: ${(error as Error).message}`,
    };
  }
  return { status: 0 };
};
