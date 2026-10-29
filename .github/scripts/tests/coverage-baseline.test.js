const assert = require('node:assert/strict');
const { test } = require('node:test');
const findBaseline = require('../coverage-baseline.js');

const miden = { name: 'miden', workflow: 'miden-coverage.yml' };
const repo = { owner: '0xMiden', repo: 'cov' };

function pushRun(id, branch, extra = {}) {
  return { id, head_branch: branch, head_sha: `${id}`.repeat(40).slice(0, 40), event: 'push',
    conclusion: 'success', ...extra };
}

function fixture({ runs = {}, artifacts = {}, base = 'next' } = {}) {
  const outputs = {};
  const listed = [];
  const github = {
    rest: {
      actions: {
        listWorkflowRuns: async (args) => {
          assert.equal(args.owner, repo.owner);
          assert.equal(args.workflow_id, 'miden-coverage.yml');
          assert.equal(args.event, 'push');
          listed.push(args.branch);
          return { data: { workflow_runs: runs[args.branch] ?? [] } };
        },
        listWorkflowRunArtifacts: async (args) => {
          assert.equal(args.name, 'miden-coverage-lcov');
          return { data: { artifacts: artifacts[args.run_id] ?? [] } };
        },
      },
    },
  };
  const context = {
    repo,
    payload: { pull_request: { base: { ref: base } }, repository: { default_branch: 'next' } },
  };
  const core = { info() {}, setOutput(key, value) { outputs[key] = value; } };
  return { options: { github, context, core }, outputs, listed };
}

const lcov = [{ name: 'miden-coverage-lcov', expired: false }];

test('takes the newest run of the base branch with an unexpired baseline', async () => {
  const { options, outputs, listed } = fixture({
    base: 'main',
    runs: { main: [pushRun(3, 'main'), pushRun(5, 'main'), pushRun(4, 'main')] },
    artifacts: { 5: [{ name: 'miden-coverage-lcov', expired: true }], 4: lcov, 3: lcov },
  });
  assert.equal((await findBaseline(options, miden)).runId, 4);
  assert.deepEqual(outputs, { run_id: '4' });
  assert.deepEqual(listed, ['main']);
});

test('falls back to the default branch', async () => {
  const { options, outputs, listed } = fixture({
    base: 'feature',
    runs: { feature: [pushRun(9, 'feature', { conclusion: 'failure' })], next: [pushRun(2, 'next')] },
    artifacts: { 9: lcov, 2: lcov },
  });
  await findBaseline(options, miden);
  assert.deepEqual(outputs, { run_id: '2' });
  assert.deepEqual(listed, ['feature', 'next']);
});

test('ignores runs of other branches and events', async () => {
  const { options, outputs } = fixture({
    runs: { next: [pushRun(7, 'other'), pushRun(8, 'next', { event: 'pull_request' })] },
    artifacts: { 7: lcov, 8: lcov },
  });
  assert.equal(await findBaseline(options, miden), null);
  assert.deepEqual(outputs, {});
});

test('rejects invalid report names and workflows', async () => {
  const { options } = fixture();
  await assert.rejects(findBaseline(options, { name: 'Bad name', workflow: 'x.yml' }));
  await assert.rejects(findBaseline(options, { name: 'miden', workflow: '../x.yml' }));
});
