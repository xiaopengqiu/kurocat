import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = new URL('../src/petInteraction.ts', import.meta.url);
const { outputText } = ts.transpileModule(readFileSync(source, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
});
const { idleDelaySeconds, SleepTransition, PetGesture, isHeadTap } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString('base64')}`
);

test('desktop idle defaults to 15 seconds and saved settings stay bounded', () => {
  assert.equal(idleDelaySeconds(undefined, false), 15);
  assert.equal(idleDelaySeconds(undefined, true), 18);
  assert.equal(idleDelaySeconds(42, false), 42);
  assert.equal(idleDelaySeconds(0, false), 5);
  assert.equal(idleDelaySeconds(1000, false), 600);
});

test('a selected stretch starts at the idle threshold and ends in sleep', () => {
  const transition = new SleepTransition(() => 0);
  assert.equal(transition.update(14_999, 15_000, true).asleep, false);
  assert.equal(transition.update(15_000, 15_000, true).asleep, false);
  assert.ok(transition.update(15_600, 15_000, true).stretch > 0.8);
  assert.equal(transition.update(16_800, 15_000, true).asleep, true);
  assert.equal(transition.update(17_000, 15_000, true).stretch, 0);
});

test('new input interrupts the stretch and quiet mode enters sleep directly', () => {
  const transition = new SleepTransition(() => 0);
  transition.update(15_000, 15_000, true);
  assert.ok(transition.update(15_600, 15_000, true).stretch > 0);
  assert.deepEqual(transition.update(0, 15_000, true), { asleep: false, stretch: 0, sleepTail: 0 });
  assert.deepEqual(transition.update(15_000, 15_000, false), { asleep: true, stretch: 0, sleepTail: 0 });
});

test('a pat has a short directional response with a tail motion', () => {
  const gesture = new PetGesture(() => 0);
  gesture.tap(100, -0.8);
  assert.ok(gesture.pose(260).lean < 0);
  assert.notEqual(gesture.pose(260).tail, 0);
  assert.equal(gesture.pose(1000).lean, 0);
});

test('a continuous stroke follows direction and adds only one accent', () => {
  const gesture = new PetGesture(() => 0);
  gesture.stroke(0, -1);
  assert.ok(gesture.pose(30).lean < 0);
  for (let time = 200; time <= 2000; time += 200) gesture.stroke(time, 1);
  assert.ok(gesture.pose(1050).tail !== 0);
  assert.equal(gesture.pose(2000).tail, 0);
  assert.ok(gesture.pose(2000).lean > 0);
  assert.equal(gesture.pose(2700).lean, 0);
});

test('a held or moved press cannot be classified as a pat', () => {
  assert.equal(isHeadTap(160, false), true);
  assert.equal(isHeadTap(160, true), false);
  assert.equal(isHeadTap(600, false), false);
});


test('a long nap has just one quiet tail flick and restarts its timer after activity', () => {
  const transition = new SleepTransition(() => 0);
  const sleepAt = 16_800;
  transition.update(sleepAt, 15_000, true);
  assert.equal(transition.update(sleepAt + 179_999, 15_000, true, true).sleepTail, 0);
  assert.notEqual(transition.update(sleepAt + 180_300, 15_000, true, true).sleepTail, 0);
  assert.equal(transition.update(sleepAt + 360_300, 15_000, true, true).sleepTail, 0);
  transition.update(100, 15_000, true);
  transition.update(sleepAt, 15_000, true);
  assert.notEqual(transition.update(sleepAt + 180_300, 15_000, true, true).sleepTail, 0);
});
