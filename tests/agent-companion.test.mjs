import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/agentCompanion.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(
  source.replace("import { catAudio } from './audio';", 'const catAudio = { playAgentNotice() {} };'),
  { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } },
);
const { AgentCompanionController } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`);

function companion(t) {
  const bubble = { dataset: {}, textContent: '', classList: new Set() };
  bubble.classList.remove = bubble.classList.delete;
  const reactions = [];
  const target = {
    onKeyDown() {}, onKeyUp() {}, triggerSleepDemo() {},
    triggerCrossDemo: () => reactions.push('angry'),
    triggerCuriousDemo: () => reactions.push('curious'),
    triggerWakeDemo: () => reactions.push('wake'),
  };
  t.mock.method(Math, 'random', () => 0);
  const before = globalThis.window;
  const timers = new Map();
  let seq = 0;
  globalThis.window = {
    setTimeout(cb, ms) { timers.set(++seq, { cb, ms }); return seq; },
    clearTimeout(id) { timers.delete(id); },
  };
  const controller = new AgentCompanionController({
    bubbleEl: bubble, getTarget: () => target, is3DTarget: () => false, allowSound: () => false,
  });
  t.after(() => { controller.dispose(); globalThis.window = before; });
  const event = (status, props = {}) => controller.handleEvent({
    kind: 'status', status, source: 'Codex', autoType: false, ...props,
  });
  return { bubble, reactions, event, timers, controller };
}

test('tool errors are curious and a subsequent task failure is still shown', t => {
  const { bubble, reactions, event, timers } = companion(t);
  event('error', { errorKind: 'tool', tool: 'shell' });
  const first = bubble.textContent;
  const initialTimer = timers.keys().next().value;
  assert.equal(bubble.dataset.tone, 'tool-error');
  assert.deepEqual(reactions, ['curious']);
  event('error', { errorKind: 'tool', tool: 'shell' });
  assert.equal(bubble.textContent, first);
  assert.deepEqual(reactions, ['curious']);
  assert.ok(timers.has(initialTimer));
  event('error', { errorKind: 'task' });
  assert.equal(bubble.dataset.tone, 'error');
  assert.deepEqual(reactions, ['curious', 'angry']);
});

test('separate occurrences rotate the captions for tool, task and recovery', t => {
  const { bubble, event } = companion(t);
  const captions = [];
  for (let i = 0; i < 2; i++) {
    event('error', { errorKind: 'tool' });
    const tool = bubble.textContent;
    event('error', { errorKind: 'task' });
    const task = bubble.textContent;
    event('thinking');
    assert.match(bubble.textContent, /思考中/);
    event('working', { text: '读取文件中…', textIsDefault: true });
    const working = bubble.textContent;
    assert.match(working, /继续|接着|重新/);
    event('working');
    assert.equal(bubble.textContent, working);
    captions.push({ tool, task, working });
  }
  for (const key of ['tool', 'task', 'working']) assert.notEqual(captions[0][key], captions[1][key]);
});

test('custom wording takes priority and the next distinct working event is ordinary', t => {
  const { bubble, event } = companion(t);
  event('error', { errorKind: 'tool', text: '构建失败，等待日志' });
  assert.match(bubble.textContent, /构建失败，等待日志$/);
  event('working', { text: '重新检查依赖' });
  assert.match(bubble.textContent, /重新检查依赖$/);
  event('working', { text: '读取文件中…', textIsDefault: true });
  assert.match(bubble.textContent, /读取文件中…$/);
});

test('a recovery caption returns to the current working message on its own', t => {
  const { bubble, event, timers } = companion(t);
  event('error', { errorKind: 'task' });
  event('working', { text: '正在运行测试', textIsDefault: true });
  assert.match(bubble.textContent, /继续|接着|重新/);
  const recovery = [...timers.values()].find(({ ms }) => ms <= 3500);
  assert.ok(recovery);
  recovery.cb();
  assert.match(bubble.textContent, /正在运行测试$/);
});

test('another source working and a completed task do not imply recovery', t => {
  const { bubble, event } = companion(t);
  event('error', { errorKind: 'task' });
  event('working', { source: 'Other' });
  assert.match(bubble.textContent, /Other · 写代码中/);
  event('done');
  event('working');
  assert.match(bubble.textContent, /Codex · 写代码中/);
});


test('recovery clears the angry pose without reopening a bubble after privacy mode', t => {
  const { bubble, reactions, event, timers, controller } = companion(t);
  event('error', { errorKind: 'task' });
  event('working');
  assert.deepEqual(reactions, ['angry', 'wake']);
  controller.setPrivacyMode(true);
  controller.setPrivacyMode(false);
  const recovery = [...timers.values()].find(({ ms }) => ms <= 3500);
  assert.ok(recovery);
  recovery.cb();
  assert.ok(bubble.classList.has('hidden'));
});
