import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../frontend/things.js', import.meta.url), 'utf8');
function load() {
  const context = vm.createContext({});
  vm.runInContext(source, context);
  return context.CloudBurritoThings;
}
const ctx = { mode: 'pinned', profile: 'demo-a', account_id: 'acct-a-fixture', region: 'eu-west-1' };

test('normalize keeps only pinned, known and complete things', () => {
  const T = load();
  const good = T.normalize({ id: 'p1', kind: 'pipeline', order: 0, context: ctx, inputs: { pipeline_name: ' synthetic-pipe ' } });
  assert.deepEqual(JSON.parse(JSON.stringify(good)), { id: 'p1', kind: 'pipeline', order: 0, context: ctx, inputs: { pipeline_name: 'synthetic-pipe' } });
  for (const raw of [
    { id: 'p2', kind: 'pipeline', order: 0, context: { mode: 'inherit' }, inputs: { pipeline_name: 'x' } },
    { id: 'p3', kind: 'ec2', order: 0, context: ctx, inputs: {} },
    { id: 'p4', kind: 'pipeline', order: 0, context: ctx, inputs: {} },
    { id: '', kind: 'pipeline', order: 0, context: ctx, inputs: { pipeline_name: 'x' } },
    { id: 'p5', kind: 'cli', order: 0, context: ctx, inputs: { command: 'rm -rf /' } },
    { id: 'p6', kind: 'stack', order: 0, context: { ...ctx, profile: 'bad\nname' }, inputs: { stack_name: 'x' } },
  ]) assert.equal(T.normalize(raw), null, JSON.stringify(raw));
});

test('row requests use only existing widgets and validated inputs', () => {
  const T = load();
  const make = (kind, inputs) => T.normalize({ id: kind, kind, order: 0, context: ctx, inputs });
  assert.deepEqual(JSON.parse(JSON.stringify(T.rowRequest(make('pipeline', { pipeline_name: 'p' })))),
    { widget: 'pipeline-runs', inputs: { pipeline_name: 'p', max_results: 5 } });
  assert.deepEqual(JSON.parse(JSON.stringify(T.rowRequest(make('package', { domain: 'd', repository: 'r', package_prefix: 'acme-core' })))),
    { widget: 'codeartifact-packages', inputs: { mode: 'list', domain: 'd', repository: 'r', package_prefix: 'acme-core', max_packages: 5 } });
  assert.equal(T.rowRequest(make('logs', { log_group: '/aws/lambda/fn', source: 'lambda' })).widget, 'log-tail');
  assert.equal(T.rowRequest(make('logs', { log_group: '/custom/group', source: 'group' })).widget, 'cloudwatch-logs');
  assert.deepEqual(JSON.parse(JSON.stringify(T.rowRequest(make('logs', { log_group: '/custom/group', source: 'group' })).inputs)),
    { mode: 'streams', log_group: '/custom/group', max_streams: 1 });
  assert.equal(T.rowRequest(make('stack', { stack_name: 's' })).widget, 'cfn-stack-detail');
  assert.equal(T.rowRequest(make('query', { log_group: '/g', query: 'fields @message', range_seconds: 3600 })).inputs.mode, 'query');
  assert.equal(T.KINDS.query.autoload, false);
  assert.equal(T.KINDS.errors.autoload, false);
  assert.equal(T.KINDS.cli.autoload, false);
});

test('summaries name the state a user needs at a glance', () => {
  const T = load();
  const pipe = T.normalize({ id: 'p', kind: 'pipeline', order: 0, context: ctx, inputs: { pipeline_name: 'p' } });
  const failed = T.summarize(pipe, { render: 'table', rows: [{ execution_id: 'e1', status: 'Failed', last_updated: '2026-11-02T14:02:00Z' }] });
  assert.equal(failed.tone, 'fail');
  assert.equal(failed.text, 'Failed');
  assert.equal(failed.at, Date.parse('2026-11-02T14:02:00Z'));
  assert.equal(T.summarize(pipe, { render: 'table', rows: [] }).text, 'No runs');
  const pkg = T.normalize({ id: 'a', kind: 'package', order: 0, context: ctx, inputs: { domain: 'd', repository: 'r', package_prefix: 'acme-core' } });
  assert.equal(T.summarize(pkg, { rows: [{ package: 'acme-core', latest_version: '4.12.0', last_published: '2026-11-02T11:02:00Z' }] }).text, '4.12.0');
  assert.equal(T.summarize(pkg, { rows: [{ package: 'acme-a' }, { package: 'acme-b' }] }).text, '2 packages');
  const stack = T.normalize({ id: 's', kind: 'stack', order: 0, context: ctx, inputs: { stack_name: 'ledger-db' } });
  const rolled = T.summarize(stack, { resources: [{}], events: [{ logical_id: 'ledger-db', status: 'UPDATE_ROLLBACK_COMPLETE', time: '2026-11-02T12:05:00Z' }] });
  assert.equal(rolled.tone, 'fail');
  assert.equal(rolled.text, 'Update rollback complete');
  const logs = T.normalize({ id: 'l', kind: 'logs', order: 0, context: ctx, inputs: { log_group: '/g', source: 'group' } });
  assert.equal(T.summarize(logs, { streams: [{ name: 's', last_event_timestamp: 1700000000000 }] }).at, 1700000000000);
});

test('groups, create and move keep a stable order inside each kind', () => {
  const T = load();
  let things = [];
  things.push(T.create('pipeline', ctx, { pipeline_name: 'a' }, things));
  things.push(T.create('pipeline', ctx, { pipeline_name: 'b' }, things));
  things.push(T.create('package', ctx, { domain: 'd', repository: 'r', package_prefix: 'p' }, things));
  assert.equal(new Set(things.map(t => t.id)).size, 3);
  assert.deepEqual(things.map(t => t.order), [0, 1, 0]);
  const groups = T.groupThings(things);
  assert.deepEqual(JSON.parse(JSON.stringify(groups.map(g => g.label))), ['Pipelines', 'Packages']);
  things = T.move(things, things[1].id, -1);
  assert.deepEqual(JSON.parse(JSON.stringify(T.groupThings(things)[0].things.map(t => t.inputs.pipeline_name))), ['b', 'a']);
  assert.deepEqual(Object.keys(T.toStored(things)[0]).sort(), ['context', 'id', 'inputs', 'kind', 'order']);
});

test('create numbers order per group, not per kind', () => {
  const T = load();
  let things = [];
  things.push(T.create('query', ctx, { log_group: '/g', query: 'fields @message', range_seconds: 3600 }, things));
  things.push(T.create('cli', ctx, { command: 'aws sts get-caller-identity' }, things));
  things.push(T.create('query', ctx, { log_group: '/g', query: 'fields @timestamp', range_seconds: 3600 }, things));
  assert.deepEqual(things.map(t => t.order), [0, 1, 2]);
});

test('old tiles become pinned things, inherited tiles use the default profile, empty tiles are skipped', () => {
  const T = load();
  const env = { defaultProfile: 'demo-a', defaultRegion: 'eu-west-1', allowedRegions: ['eu-west-1', 'us-east-1'],
    profiles: [{ name: 'demo-a', account_id: 'acct-a-fixture', region: 'eu-west-1' }] };
  const pinB = { profile: 'demo-b', account_id: 'acct-b-fixture', region: 'eu-west-1' };
  const tiles = [
    { id: 'pipeline-runs', widget: 'pipeline-runs', x: 0, y: 0, w: 4, h: 3, config: { inputs: { pinned_pipelines: [{ ...pinB, pipeline_name: 'synthetic-pipe' }, { ...pinB, pipeline_name: 'synthetic-pipe' }] } } },
    { id: 'codeartifact-packages', widget: 'codeartifact-packages', x: 0, y: 3, w: 6, h: 4, config: { context: { mode: 'inherit' }, inputs: { domain: 'd', repository: 'r', package_prefix: 'acme', max_packages: 50 } } },
    { id: 'aws-cli:1', x: 0, y: 7, w: 6, h: 4, config: { inputs: { command: 'aws sts get-caller-identity', pinned_cli_commands: [{ id: 'x', ...pinB, command: 'aws cloudformation list-stacks' }] } } },
    { id: 'cfn-stacks', widget: 'cfn-stacks', x: 6, y: 0, w: 6, h: 3, config: {} },
  ];
  const result = T.migrateTiles(tiles, env);
  assert.equal(result.converted, 3);
  assert.equal(result.skippedEmpty, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(result.things.map(t => [t.kind, t.context.profile, T.name(t)]))), [
    ['pipeline', 'demo-b', 'synthetic-pipe'],
    ['package', 'demo-a', 'acme'],
    ['cli', 'demo-b', 'aws cloudformation list-stacks'],
    ['cli', 'demo-a', 'aws sts get-caller-identity'],
  ]);
  const noDefault = T.migrateTiles(tiles, { ...env, defaultProfile: '' });
  // The inherited CodeArtifact tile cannot get an account; the CLI tile still
  // converts its pinned command, so it counts as converted.
  assert.equal(noDefault.skippedNoAccount, 1);
  assert.equal(noDefault.converted, 2);
  assert.equal(noDefault.things.filter(t => t.context.profile === 'demo-a').length, 0);
  assert.equal(T.migrateTiles([tiles[1]], { ...env, defaultProfile: '' }).skippedNoAccount, 1);
});

test('the login command quotes a profile name only when the shell needs it', () => {
  const T = load();
  assert.equal(T.loginCommand('demo-a'), 'aws sso login --profile demo-a');
  assert.equal(T.loginCommand('demo.team_a@acme'), 'aws sso login --profile demo.team_a@acme');
  assert.equal(T.loginCommand('demo a'), "aws sso login --profile 'demo a'");
  assert.equal(T.loginCommand("demo-a; echo it's"), "aws sso login --profile 'demo-a; echo it'\\''s'");
  assert.equal(T.loginCommand('$(demo)'), "aws sso login --profile '$(demo)'");
});
