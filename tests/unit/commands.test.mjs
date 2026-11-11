import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const things = readFileSync(new URL('../../frontend/things.js', import.meta.url), 'utf8');
const commands = readFileSync(new URL('../../frontend/commands.js', import.meta.url), 'utf8');
function load() {
  const context = vm.createContext({});
  vm.runInContext(things, context);
  vm.runInContext(commands, context);
  return context.CloudBurritoCommands;
}
const env = {
  profiles: [{ name: 'prod-core', account_id: 'acct-core-fixture', region: 'eu-west-1' },
    { name: 'shared-tools', account_id: 'acct-tools-fixture', region: 'us-east-1' }],
  defaultProfile: 'prod-core', defaultRegion: 'eu-west-1', allowedRegions: ['eu-west-1', 'us-east-1'],
  packageRepos: [{ profile: 'shared-tools', domain: 'acme', repository: 'pypi-internal' }],
  names: { pipeline: ['payments-api', 'payroll'], stack: [], package: [] },
};
const plain = value => JSON.parse(JSON.stringify(value));

test('thing commands resolve a pinned account from @profile, the default profile and a region', () => {
  const C = load();
  assert.deepEqual(plain(C.parse(':pipe payments-api @shared-tools', env)), { ok: true, type: 'thing', kind: 'pipeline',
    context: { mode: 'pinned', profile: 'shared-tools', account_id: 'acct-tools-fixture', region: 'us-east-1' },
    inputs: { pipeline_name: 'payments-api' } });
  assert.equal(C.parse('pipe payments-api', env).context.profile, 'prod-core');
  assert.equal(C.parse('pipe payments-api us-east-1', env).context.region, 'us-east-1');
  assert.match(C.parse('pipe payments-api @nobody', env).error, /Unknown profile @nobody/);
  assert.match(C.parse('pipe x', { ...env, defaultProfile: '' }).error, /Add @profile/);
  assert.match(C.parse('pipe', env).error, /pipeline name/);
});

test('each kind builds validated inputs', () => {
  const C = load();
  assert.deepEqual(plain(C.parse('pkg acme-core acme/pypi-internal', env).inputs), { domain: 'acme', repository: 'pypi-internal', package_prefix: 'acme-core' });
  assert.deepEqual(plain(C.parse('pkg acme-core @shared-tools', env).inputs), { domain: 'acme', repository: 'pypi-internal', package_prefix: 'acme-core' });
  assert.match(C.parse('pkg acme-core', env).error, /domain\/repository/);
  assert.deepEqual(plain(C.parse('logs payments-handler', env).inputs), { log_group: '/aws/lambda/payments-handler', source: 'lambda', function_name: 'payments-handler' });
  assert.deepEqual(plain(C.parse('logs /custom/group', env).inputs), { log_group: '/custom/group', source: 'group' });
  assert.deepEqual(plain(C.parse('stacks ledger-', env).inputs), { name_prefix: 'ledger-' });
  assert.deepEqual(plain(C.parse('errors /aws/lambda/ 12', env).inputs), { hours: 12, log_group_pattern: '/aws/lambda/' });
  assert.deepEqual(plain(C.parse('insights /aws/lambda/fn "fields @message | limit 5" 3h', env).inputs),
    { log_group: '/aws/lambda/fn', query: 'fields @message | limit 5', range_seconds: 10800 });
  assert.match(C.parse('insights /g "q" 2w', env).error, /15m, 1h/);
  assert.match(C.parse('insights /g "unterminated', env).error, /quote/);
});

test('aws commands keep their own arguments and only a @profile is ours', () => {
  const C = load();
  const parsed = C.parse(`aws lambda list-functions --function-version ALL --master-region us-east-1 --query "Functions[?Runtime=='python3.12']" @shared-tools`, env);
  assert.equal(parsed.kind, 'cli');
  assert.equal(parsed.context.profile, 'shared-tools');
  assert.equal(parsed.inputs.command, `aws lambda list-functions --function-version ALL --master-region us-east-1 --query "Functions[?Runtime=='python3.12']"`);
  assert.match(C.parse('aws', env).error, /full command/);
});

test('app commands and completion', () => {
  const C = load();
  assert.deepEqual(plain(C.parse(':settings', env)), { ok: true, type: 'app', name: 'settings' });
  assert.match(C.parse('deploy everything', env).error, /Unknown command deploy/);
  assert.deepEqual(plain(C.complete('pipe pay @sh', env).items.map(i => i.value)), ['@shared-tools']);
  assert.deepEqual(plain(C.complete(':pi', env).items.map(i => i.value)), [':pipe ']);
  assert.deepEqual(plain(C.complete('pipe pay', env).items.map(i => i.value)), ['payments-api', 'payroll']);
  assert.equal(C.complete('pipe pay', env).from, 5);
});

test('aws text is passed through exactly, backslashes and both quote types included', () => {
  const C = load();
  const backslash = C.parse('aws logs describe-log-groups --log-group-name-prefix a\\b', env);
  assert.equal(backslash.inputs.command, 'aws logs describe-log-groups --log-group-name-prefix a\\b');
  assert.equal(backslash.context.profile, 'prod-core');
  const quotes = C.parse(`aws logs filter-log-events --filter-pattern a"b'c'd`, env);
  assert.equal(quotes.inputs.command, `aws logs filter-log-events --filter-pattern a"b'c'd`);
});

test('only a trailing unquoted @profile is taken from aws text', () => {
  const C = load();
  const quoted = C.parse('aws sts get-caller-identity "@x"', env);
  assert.equal(quoted.context.profile, 'prod-core');
  assert.equal(quoted.inputs.command, 'aws sts get-caller-identity "@x"');
  const middle = C.parse('aws sts get-caller-identity @x --foo @shared-tools', env);
  assert.equal(middle.context.profile, 'shared-tools');
  assert.equal(middle.inputs.command, 'aws sts get-caller-identity @x --foo');
  assert.match(C.parse('aws @shared-tools', env).error, /full command/);
});
