import { issueBody, reasonLines, resolution } from './findings.js';

/**
 * The label on every issue the watch opens. It finds its issues by it, so
 * removing it from one makes the watch open another.
 */
export const LABEL = 'platform-watch';

const KEY = /<!-- platform-watch:(\S+) -->/;
const REASON = /<!-- platform-watch-reason:(\S+) -->/g;

/**
 * A small GitHub REST client, with only the calls the watch makes.
 *
 * @param {object} options
 * @param {string} options.repo `owner/name`.
 * @param {string} [options.token] Needed for anything but reading a public
 *   repository.
 * @param {typeof fetch} options.fetch
 */
export const gitHub = ({ repo, token, fetch }) => {
  const request = async (method, endpoint, body) => {
    const headers = {
      accept: 'application/vnd.github+json',
      ...(token && { authorization: `Bearer ${token}` }),
    };
    // Only writes have a body, and fetch rejects one on a GET.
    const response = await fetch(
      `https://api.github.com${endpoint}`,
      body
        ? {
            method,
            headers: { ...headers, 'content-type': 'application/json' },
            body: JSON.stringify(body),
          }
        : { method, headers },
    );
    if (!response.ok) {
      throw new Error(
        `GitHub: ${method} ${endpoint}: HTTP ${response.status}: ${await response.text()}`,
      );
    }
    return response.json();
  };
  const issue = `/repos/${repo}/issues`;

  return {
    /** Every issue with the label, open or closed, newest first. */
    issues: async () => {
      const all = [];
      for (let page = 1; ; page++) {
        const batch = await request(
          'GET',
          `${issue}?labels=${LABEL}&state=all&sort=created&direction=desc&per_page=100&page=${page}`,
        );
        // The issues endpoint lists pull requests too.
        all.push(...batch.filter((item) => !item.pull_request));
        if (batch.length < 100) {
          return all;
        }
      }
    },
    comments: (number) =>
      request('GET', `${issue}/${number}/comments?per_page=100`),
    create: (title, body) =>
      request('POST', issue, { title, body, labels: [LABEL] }),
    comment: (number, body) =>
      request('POST', `${issue}/${number}/comments`, { body }),
    update: (number, fields) => request('PATCH', `${issue}/${number}`, fields),
  };
};

// Closed as completed means the work was done, so a finding that's back
// (a source broke again) is new work and gets a new issue. Closed as not
// planned or as a duplicate is a decision about this issue, which only a
// new reason, such as a store deadline, reopens.
const done = (issue) =>
  issue.state === 'closed' && issue.state_reason === 'completed';

/**
 * What to change on GitHub so its issues match the findings: open an issue
 * for new work, comment on (or reopen) one when there's a new reason for it,
 * and close the open ones whose work is done. Only reads from GitHub.
 *
 * @param {object[]} found From findings().
 * @param {Map<string, import('./read.js').Reading>} readings
 * @param {ReturnType<typeof gitHub>} github
 * @returns {Promise<object[]>} Actions for apply() and summarize().
 */
export const plan = async (found, readings, github) => {
  // The newest issue for each finding. An older one with the same key is
  // history: closed as done before the finding came back.
  const latest = new Map();
  for (const issue of await github.issues()) {
    const key = issue.body?.match(KEY)?.[1];
    if (key && !latest.has(key)) {
      latest.set(key, issue);
    }
  }

  const actions = [];
  for (const finding of found) {
    const issue = latest.get(finding.key);
    if (!issue || done(issue)) {
      actions.push({ type: 'open', finding });
      continue;
    }
    const comments = await github.comments(issue.number);
    const seen = new Set(
      [issue.body, ...comments.map((comment) => comment.body)].flatMap((body) =>
        [...body.matchAll(REASON)].map((match) => match[1]),
      ),
    );
    const reasons = finding.reasons.filter((reason) => !seen.has(reason.id));
    if (reasons.length > 0) {
      actions.push({
        type: issue.state === 'open' ? 'comment' : 'reopen',
        issue,
        body: reasonLines(reasons),
      });
    }
  }

  const current = new Set(found.map((finding) => finding.key));
  for (const [key, issue] of latest) {
    if (issue.state !== 'open' || current.has(key)) {
      continue;
    }
    const body = resolution(key, readings);
    if (body) {
      actions.push({ type: 'close', issue, body });
    }
  }
  return actions;
};

/** Makes one change plan() asked for. */
export const apply = async (action, github) => {
  switch (action.type) {
    case 'open':
      await github.create(action.finding.title, issueBody(action.finding));
      return;
    case 'comment':
      await github.comment(action.issue.number, action.body);
      return;
    case 'reopen':
      await github.comment(action.issue.number, action.body);
      await github.update(action.issue.number, { state: 'open' });
      return;
    case 'close':
      await github.comment(action.issue.number, action.body);
      await github.update(action.issue.number, {
        state: 'closed',
        state_reason: 'completed',
      });
      return;
  }
};

/** @returns {string} One line for the log. */
export const summarize = (action) =>
  action.type === 'open'
    ? `open an issue: ${action.finding.title}`
    : `${action.type} #${action.issue.number}: ${action.issue.title}`;
