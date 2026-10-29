// Finds the baseline of a coverage report for a pull request: the LCOV artifact of the newest
// successful push run of the report's workflow on the pull request's base branch, or on the
// default branch when the base branch has none (a stacked pull request, or no run yet).

// Push runs are checked newest first; a baseline older than this many runs is not looked for.
const RUNS = 20;

function check(value, pattern, what) {
  if (typeof value !== 'string' || !pattern.test(value)) throw new Error(`Invalid ${what}`);
  return value;
}

async function newestBaseline({ github, context }, artifactName, workflow, branch) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    ...context.repo, workflow_id: workflow, branch, event: 'push', status: 'success',
    per_page: RUNS,
  });
  const runs = data.workflow_runs
    .filter(run => run.head_branch === branch && run.event === 'push' &&
      run.conclusion === 'success')
    .sort((left, right) => right.id - left.id);
  for (const run of runs) {
    const { data: artifacts } = await github.rest.actions.listWorkflowRunArtifacts({
      ...context.repo, run_id: run.id, name: artifactName, per_page: 100,
    });
    if (artifacts.artifacts.some(artifact => artifact.name === artifactName && !artifact.expired)) {
      return { runId: run.id, sha: run.head_sha };
    }
  }
  return null;
}

// Sets the `run_id` output to the run holding the `<name>-coverage-lcov` artifact, or leaves it
// unset when there is no baseline: the report is then rendered for the head only.
async function findBaseline(options, report) {
  const name = check(report?.name, /^[a-z][a-z0-9-]*$/, 'coverage report name');
  const workflow = check(report?.workflow, /^[A-Za-z0-9._-]+\.yml$/, 'coverage workflow');
  const { context, core } = options;
  const base = context.payload.pull_request?.base?.ref;
  const fallback = context.payload.repository?.default_branch;
  const artifactName = `${name}-coverage-lcov`;
  for (const branch of [...new Set([base, fallback])].filter(Boolean)) {
    const found = await newestBaseline(options, artifactName, workflow, branch);
    if (found) {
      core.info(`Baseline from run ${found.runId} on ${branch} at ${found.sha}`);
      core.setOutput('run_id', String(found.runId));
      return found;
    }
  }
  core.info('No baseline found');
  return null;
}

module.exports = findBaseline;
