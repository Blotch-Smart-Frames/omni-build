import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PLATFORMS } from '../../omni-build/src/platforms.ts';
import { SUMS, archiveName } from './archives.ts';
import { formula, formulaText } from './formula.ts';

/** A made-up SHA-256 for each platform's archive. */
const SUMS_BY_NAME = new Map(
  Object.keys(PLATFORMS).map((key, index) => [
    archiveName(key),
    String(index).repeat(64),
  ]),
);

let root: string;
let archives: string;

beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), 'omni-build-formula-test-'));
  archives = path.join(root, 'archives');
  mkdirSync(path.join(root, 'packages', 'omni-build'), { recursive: true });
  mkdirSync(archives);
  writeFileSync(
    path.join(root, 'packages', 'omni-build', 'package.json'),
    JSON.stringify({
      name: '@blotch/omni-build',
      version: '1.2.3',
      license: 'MIT',
      repository: {
        type: 'git',
        url: 'git+https://github.com/Blotch-Smart-Frames/omni-build.git',
        directory: 'packages/omni-build',
      },
    }),
  );
  writeFileSync(
    path.join(archives, SUMS),
    [...SUMS_BY_NAME].map(([name, sum]) => `${sum}  ${name}\n`).join(''),
  );
});

afterEach(() => {
  rmSync(root, { recursive: true, force: true });
});

describe('formulaText', () => {
  it("installs each macOS and Linux archive's omni, pinned by its SHA-256", () => {
    expect(
      formulaText({
        version: '1.2.3',
        homepage: 'https://example.com/omni-build',
        license: 'MIT',
        url: 'https://example.com/download',
        sums: SUMS_BY_NAME,
      }),
    ).toBe(`# Written by OmniBuild's release workflow for each version, from
# packages/release/src/formula.ts in https://example.com/omni-build:
# change it there.
class OmniBuild < Formula
  desc "Build, sign, install and ship iOS and Android apps with no native toolchain"
  homepage "https://example.com/omni-build"
  version "1.2.3"
  license "MIT"

  on_macos do
    on_arm do
      url "https://example.com/download/omni-build-aarch64-apple-darwin.tar.gz"
      sha256 "${'0'.repeat(64)}"
    end
    on_intel do
      url "https://example.com/download/omni-build-x86_64-apple-darwin.tar.gz"
      sha256 "${'1'.repeat(64)}"
    end
  end

  on_linux do
    on_arm do
      url "https://example.com/download/omni-build-aarch64-unknown-linux-musl.tar.gz"
      sha256 "${'2'.repeat(64)}"
    end
    on_intel do
      url "https://example.com/download/omni-build-x86_64-unknown-linux-musl.tar.gz"
      sha256 "${'3'.repeat(64)}"
    end
  end

  def install
    bin.install "omni"
  end

  test do
    assert_equal "omni #{version}", shell_output("#{bin}/omni --version").strip
  end
end
`);
  });

  it('says which archive has no SHA-256', () => {
    expect(() =>
      formulaText({
        version: '1.2.3',
        homepage: 'https://example.com',
        license: 'MIT',
        url: 'https://example.com',
        sums: new Map(),
      }),
    ).toThrow("There's no SHA-256 for omni-build-aarch64-apple-darwin.tar.gz.");
  });
});

describe('formula', () => {
  it("downloads from the version's GitHub release, without a URL", () => {
    const out = path.join(root, 'tap', 'Formula', 'omni-build.rb');
    const log = vi.fn();

    formula({ archives, out, root, log });

    const text = readFileSync(out, 'utf8');
    expect(text).toContain(
      'homepage "https://github.com/Blotch-Smart-Frames/omni-build"',
    );
    expect(text).toContain('version "1.2.3"');
    expect(text).toContain(
      'url "https://github.com/Blotch-Smart-Frames/omni-build/releases/download/v1.2.3/omni-build-x86_64-apple-darwin.tar.gz"',
    );
    expect(log).toHaveBeenCalledWith(
      `Wrote the formula for omni-build 1.2.3 to ${out}`,
    );
  });

  it('downloads from the URL it gets, such as a folder for CI', () => {
    const out = path.join(root, 'omni-build.rb');

    formula({
      archives,
      out,
      url: 'file:///tmp/archives/',
      root,
      log: () => {},
    });

    expect(readFileSync(out, 'utf8')).toContain(
      'url "file:///tmp/archives/omni-build-x86_64-apple-darwin.tar.gz"',
    );
  });

  it('logs to the console by default', () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {});

    formula({ archives, out: path.join(root, 'omni-build.rb'), root });

    expect(log).toHaveBeenCalledOnce();
    log.mockRestore();
  });
});
