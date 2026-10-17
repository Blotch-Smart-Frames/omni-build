import { describe, expect, it, vi } from 'vitest';

import { marker, reasonMarker } from './findings.js';
import { LABEL, apply, gitHub, plan, summarize } from './issues.js';
import { SOURCES } from './sources.js';

const finding = (key, reasonIds = ['released']) => ({
  key,
  title: `Title of ${key}`,
  context: 'Context:',
  reasons: reasonIds.map((id) => ({ id, text: `Reason ${id}` })),
  why: 'Why.',
  todo: ['Do it.'],
  doneWhen: "it's done",
});

let nextNumber = 100;
const issue = (key, fields = {}) => ({
  number: nextNumber--,
  title: `Title of ${key}`,
  body: `${marker(key)}\n\n- Reason released ${reasonMarker('released')}`,
  state: 'open',
  state_reason: null,
  ...fields,
});

/** A GitHub client with these issues, and comments by issue number. */
const fakeGitHub = (issues = [], comments = {}) => ({
  issues: vi.fn(async () => issues),
  comments: vi.fn(async (number) => comments[number] ?? []),
  create: vi.fn(async () => ({})),
  comment: vi.fn(async () => ({})),
  update: vi.fn(async () => ({})),
});

// The shells up to date and every source readable, so resolution() closes
// whatever is open.
const resolvedReadings = new Map(
  SOURCES.map((source) => [
    source.id,
    {
      source,
      ok: true,
      value: {
        major: 99,
        version: '99.0',
        compileSdk: 99,
        targetSdk: 99,
      },
    },
  ]),
);

describe('plan', () => {
  it('opens an issue for a new finding', async () => {
    const found = [finding('ios-xcode-27')];

    expect(await plan(found, resolvedReadings, fakeGitHub())).toEqual([
      { type: 'open', finding: found[0] },
    ]);
  });

  it('leaves an open issue alone when it has every reason', async () => {
    const github = fakeGitHub([issue('ios-xcode-27')]);

    expect(
      await plan([finding('ios-xcode-27')], resolvedReadings, github),
    ).toEqual([]);
  });

  it('comments with only the new reasons', async () => {
    const open = issue('ios-xcode-27');
    const github = fakeGitHub([open]);

    const actions = await plan(
      [finding('ios-xcode-27', ['released', 'required'])],
      resolvedReadings,
      github,
    );

    expect(actions).toEqual([
      {
        type: 'comment',
        issue: open,
        body: `- Reason required ${reasonMarker('required')}`,
      },
    ]);
    expect(github.comments).toHaveBeenCalledWith(open.number);
  });

  it('counts the reasons in comments', async () => {
    const open = issue('ios-xcode-27');
    const github = fakeGitHub([open], {
      [open.number]: [
        { body: `- Reason required ${reasonMarker('required')}` },
      ],
    });

    expect(
      await plan(
        [finding('ios-xcode-27', ['released', 'required'])],
        resolvedReadings,
        github,
      ),
    ).toEqual([]);
  });

  it('reopens an issue closed as not planned for a new reason', async () => {
    const closed = issue('ios-xcode-27', {
      state: 'closed',
      state_reason: 'not_planned',
    });

    const actions = await plan(
      [finding('ios-xcode-27', ['released', 'required'])],
      resolvedReadings,
      fakeGitHub([closed]),
    );

    expect(actions).toEqual([
      {
        type: 'reopen',
        issue: closed,
        body: `- Reason required ${reasonMarker('required')}`,
      },
    ]);
  });

  it('leaves an issue closed as not planned closed otherwise', async () => {
    const closed = issue('watch-android-sdk', {
      state: 'closed',
      state_reason: 'not_planned',
      body: `${marker('watch-android-sdk')} ${reasonMarker('unreadable')}`,
    });

    expect(
      await plan(
        [finding('watch-android-sdk', ['unreadable'])],
        resolvedReadings,
        fakeGitHub([closed]),
      ),
    ).toEqual([]);
  });

  it('opens a new issue when work closed as done is needed again', async () => {
    const found = [finding('watch-android-sdk', ['unreadable'])];
    const github = fakeGitHub([
      issue('watch-android-sdk', {
        state: 'closed',
        state_reason: 'completed',
      }),
    ]);

    expect(await plan(found, resolvedReadings, github)).toEqual([
      { type: 'open', finding: found[0] },
    ]);
  });

  it('goes by the newest issue for a finding', async () => {
    // Newest first, as the API lists them.
    const github = fakeGitHub([
      issue('ios-xcode-27'),
      issue('ios-xcode-27', { state: 'closed', state_reason: 'completed' }),
    ]);

    expect(
      await plan([finding('ios-xcode-27')], resolvedReadings, github),
    ).toEqual([]);
  });

  it("ignores labelled issues it didn't open", async () => {
    const found = [finding('ios-xcode-27')];
    const github = fakeGitHub([
      issue('x', { body: null }),
      issue('x', { body: 'Written by hand' }),
    ]);

    expect(await plan(found, resolvedReadings, github)).toEqual([
      { type: 'open', finding: found[0] },
    ]);
  });

  it('closes an open issue whose work is done', async () => {
    const open = issue('ios-xcode-27');

    expect(await plan([], resolvedReadings, fakeGitHub([open]))).toEqual([
      {
        type: 'close',
        issue: open,
        body: 'The iOS shells now build with Xcode 99.0, so this is done.',
      },
    ]);
  });

  it('leaves an open issue open when it cannot tell the work is done', async () => {
    const github = fakeGitHub([issue('written-by-someone-else')]);

    expect(await plan([], resolvedReadings, github)).toEqual([]);
  });

  it('leaves closed issues closed', async () => {
    const github = fakeGitHub([
      issue('ios-xcode-27', { state: 'closed', state_reason: 'completed' }),
    ]);

    expect(await plan([], resolvedReadings, github)).toEqual([]);
  });
});

describe('apply', () => {
  const open = issue('ios-xcode-27');

  it('opens an issue with the full body', async () => {
    const github = fakeGitHub();

    await apply({ type: 'open', finding: finding('ios-xcode-27') }, github);

    expect(github.create).toHaveBeenCalledWith(
      'Title of ios-xcode-27',
      expect.stringContaining(marker('ios-xcode-27')),
    );
  });

  it('comments', async () => {
    const github = fakeGitHub();

    await apply({ type: 'comment', issue: open, body: 'New' }, github);

    expect(github.comment).toHaveBeenCalledWith(open.number, 'New');
    expect(github.update).not.toHaveBeenCalled();
  });

  it('comments, then reopens', async () => {
    const github = fakeGitHub();

    await apply({ type: 'reopen', issue: open, body: 'New' }, github);

    expect(github.comment).toHaveBeenCalledWith(open.number, 'New');
    expect(github.update).toHaveBeenCalledWith(open.number, { state: 'open' });
  });

  it('comments, then closes as completed', async () => {
    const github = fakeGitHub();

    await apply({ type: 'close', issue: open, body: 'Done' }, github);

    expect(github.comment).toHaveBeenCalledWith(open.number, 'Done');
    expect(github.update).toHaveBeenCalledWith(open.number, {
      state: 'closed',
      state_reason: 'completed',
    });
  });
});

describe('summarize', () => {
  it('names the issue to open', () => {
    expect(summarize({ type: 'open', finding: finding('ios-xcode-27') })).toBe(
      'open an issue: Title of ios-xcode-27',
    );
  });

  it('names the issue to change', () => {
    expect(
      summarize({ type: 'close', issue: { number: 7, title: 'Old' } }),
    ).toBe('close #7: Old');
  });
});

describe('gitHub', () => {
  const json = (body, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'content-type': 'application/json' },
    });

  it('lists every labelled issue, page by page, without pull requests', async () => {
    const first = Array.from({ length: 100 }, (_, i) => ({ number: i }));
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(json(first))
      .mockResolvedValueOnce(
        json([{ number: 100 }, { number: 101, pull_request: {} }]),
      );

    const issues = await gitHub({ repo: 'o/r', token: 't', fetch }).issues();

    expect(issues.map(({ number }) => number)).toEqual([
      ...first.map(({ number }) => number),
      100,
    ]);
    expect(fetch.mock.calls.map(([url]) => url)).toEqual([
      `https://api.github.com/repos/o/r/issues?labels=${LABEL}&state=all&sort=created&direction=desc&per_page=100&page=1`,
      `https://api.github.com/repos/o/r/issues?labels=${LABEL}&state=all&sort=created&direction=desc&per_page=100&page=2`,
    ]);
    expect(fetch.mock.calls[0][1]).toEqual({
      method: 'GET',
      headers: {
        accept: 'application/vnd.github+json',
        authorization: 'Bearer t',
      },
    });
  });

  it('reads without a token', async () => {
    const fetch = vi.fn(async () => json([]));

    await gitHub({ repo: 'o/r', fetch }).comments(3);

    expect(fetch).toHaveBeenCalledWith(
      'https://api.github.com/repos/o/r/issues/3/comments?per_page=100',
      {
        method: 'GET',
        headers: { accept: 'application/vnd.github+json' },
      },
    );
  });

  it.each([
    [
      'create',
      ['Title', 'Body'],
      'POST',
      '/repos/o/r/issues',
      { title: 'Title', body: 'Body', labels: [LABEL] },
    ],
    [
      'comment',
      [3, 'Body'],
      'POST',
      '/repos/o/r/issues/3/comments',
      { body: 'Body' },
    ],
    [
      'update',
      [3, { state: 'open' }],
      'PATCH',
      '/repos/o/r/issues/3',
      { state: 'open' },
    ],
  ])('%s sends JSON', async (name, args, method, endpoint, body) => {
    const fetch = vi.fn(async () => json({}));

    await gitHub({ repo: 'o/r', token: 't', fetch })[name](...args);

    expect(fetch).toHaveBeenCalledWith(`https://api.github.com${endpoint}`, {
      method,
      headers: {
        accept: 'application/vnd.github+json',
        authorization: 'Bearer t',
        'content-type': 'application/json',
      },
      body: JSON.stringify(body),
    });
  });

  it('says what GitHub refused', async () => {
    const fetch = vi.fn(async () =>
      json({ message: 'Resource not accessible by integration' }, 403),
    );

    await expect(
      gitHub({ repo: 'o/r', token: 't', fetch }).comment(3, 'Body'),
    ).rejects.toThrow(
      'GitHub: POST /repos/o/r/issues/3/comments: HTTP 403: {"message":"Resource not accessible by integration"}',
    );
  });
});
