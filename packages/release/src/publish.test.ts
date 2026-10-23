import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { INDEX } from './pack.ts';
import type { Exec } from './process.ts';
import { publish } from './publish.ts';

let packages: string;
/** Each npm command publish ran. */
let commands: string[];

/** The index `pack` would leave, the launcher first to show it goes last. */
const packed = (
  version = '1.2.3',
  names = [
    '@blotch/omni-build',
    '@blotch/omni-build-linux-x64',
    '@blotch/omni-build-darwin-arm64',
  ],
) =>
  writeFileSync(
    path.join(packages, INDEX),
    JSON.stringify(
      names.map((name) => ({
        file: `${name.slice(1).replace('/', '-')}-${version}.tgz`,
        manifest: { name, version },
      })),
    ),
  );

/** npm, with `onNpm` the versions it has of each package it has. */
const npm =
  (onNpm: Record<string, string> = {}): Exec =>
  async (_node, [, command, ...args]) => {
    commands.push([command, ...args].join(' '));
    if (command !== 'view') {
      return '';
    }
    const [name, version] = args[0].split(/(?<=.)@/);
    if (!(name in onNpm)) {
      throw new Error('npm error code E404');
    }
    return onNpm[name] === version ? `${version}\n` : '';
  };

const publishing = (exec: Exec, tag?: string) => {
  const logged: string[] = [];
  return publish({
    packages,
    tag,
    exec,
    log: (line) => logged.push(line),
  }).then(() => logged);
};

beforeEach(() => {
  packages = mkdtempSync(path.join(tmpdir(), 'omni-build-publish-test-'));
  commands = [];
});

afterEach(() => {
  rmSync(packages, { recursive: true, force: true });
});

describe('publish', () => {
  it('publishes the platforms, then the launcher, as public packages', async () => {
    packed();

    const logged = await publishing(npm(), 'v1.2.3');

    const tarball = (name: string) => path.join(packages, `${name}-1.2.3.tgz`);
    expect(commands).toEqual([
      'view @blotch/omni-build-linux-x64@1.2.3 version',
      `publish ${tarball('blotch-omni-build-linux-x64')} --access public --tag latest`,
      'view @blotch/omni-build-darwin-arm64@1.2.3 version',
      `publish ${tarball('blotch-omni-build-darwin-arm64')} --access public --tag latest`,
      'view @blotch/omni-build@1.2.3 version',
      `publish ${tarball('blotch-omni-build')} --access public --tag latest`,
    ]);
    expect(logged).toEqual([
      'Published @blotch/omni-build-linux-x64@1.2.3',
      'Published @blotch/omni-build-darwin-arm64@1.2.3',
      'Published @blotch/omni-build@1.2.3',
    ]);
  });

  it('leaves what npm has already, as after a release that stopped partway', async () => {
    packed();

    const logged = await publishing(
      npm({
        '@blotch/omni-build-linux-x64': '1.2.3',
        '@blotch/omni-build': '1.2.2',
      }),
    );

    expect(logged).toEqual([
      '@blotch/omni-build-linux-x64@1.2.3 is on npm already',
      'Published @blotch/omni-build-darwin-arm64@1.2.3',
      'Published @blotch/omni-build@1.2.3',
    ]);
  });

  it('keeps a prerelease off latest', async () => {
    packed('2.0.0-beta.1');

    await publishing(npm());

    const published = commands.filter((command) =>
      command.startsWith('publish'),
    );
    expect(published).toHaveLength(3);
    for (const command of published) {
      expect(command).toMatch(/ --tag next$/);
    }
  });

  it('publishes only for the tag of its version', async () => {
    packed();

    await expect(publishing(npm(), 'v1.2.4')).rejects.toThrow(
      "The tag v1.2.4 isn't the packages' version, 1.2.3: a release of it is tagged v1.2.3.",
    );
    expect(commands).toEqual([]);
  });

  it('needs the launcher', async () => {
    packed('1.2.3', ['@blotch/omni-build-linux-x64']);

    await expect(publishing(npm())).rejects.toThrow(
      `There's no @blotch/omni-build package in ${packages}.`,
    );
    expect(commands).toEqual([]);
  });
});
