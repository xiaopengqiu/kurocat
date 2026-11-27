import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = new URL('../src/petInteraction.ts', import.meta.url);
const { outputText } = ts.transpileModule(readFileSync(source, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 },
});
const { idleDelaySeconds, SleepTransition, PetGesture, isPetTap, bendTail, pawSideAt, isWakeBurst, gentleWakeAmount, taskFatiguePose } = await import(
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
  assert.ok(gesture.pose(260).tail);
  assert.equal(gesture.pose(1000).lean, 0);
});

test('a continuous stroke follows direction and adds only one accent', () => {
  const gesture = new PetGesture(() => 0);
  gesture.stroke(0, -1);
  assert.ok(gesture.pose(30).lean < 0);
  for (let time = 200; time <= 2000; time += 200) gesture.stroke(time, 1);
  assert.ok(gesture.pose(1050).tail);
  assert.ok(gesture.pose(2000).tail);
  assert.equal(gesture.pose(3000).tail, null);
  assert.ok(gesture.pose(2000).lean > 0);
  assert.equal(gesture.pose(2700).lean, 0);
});

test('a held or moved press cannot be classified as a pat', () => {
  assert.equal(isPetTap(160, false), true);
  assert.equal(isPetTap(160, true), false);
  assert.equal(isPetTap(600, false), false);
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


test('a tail sweep travels through the curve while preserving its root and segment lengths', () => {
  const rest = [{ x: 0, y: 0 }, { x: 34, y: 4 }, { x: 38, y: -34 }, { x: 70, y: -35 }];
  const curveLength = (points) => {
    let length = 0;
    let previous = points[0];
    for (let step = 1; step <= 64; step++) {
      const t = step / 64;
      const u = 1 - t;
      const weights = [u ** 3, 3 * u ** 2 * t, 3 * u * t ** 2, t ** 3];
      const x = points.reduce((sum, point, i) => sum + point.x * weights[i], 0);
      const y = points.reduce((sum, point, i) => sum + point.y * weights[i], 0);
      length += Math.hypot(x - previous.x, y - previous.y);
      previous = { x, y };
    }
    return length;
  };
  const restingLength = curveLength(rest);
  const poseAt = (elapsedMs) => {
    const points = rest.map(point => ({ ...point }));
    bendTail(points, { elapsedMs, durationMs: 1550, strength: 0.95, direction: 1 });
    return points;
  };
  const early = poseAt(100);
  assert.notDeepEqual(early[1], rest[1]);
  assert.ok(Math.abs((early[3].y - early[2].y) - (rest[3].y - rest[2].y)) < 1e-8);
  for (const elapsedMs of [100, 450, 750, 1250, 1800]) {
    const points = poseAt(elapsedMs);
    assert.deepEqual(points[0], rest[0]);
    assert.ok(Math.abs(curveLength(points) / restingLength - 1) < 0.05);
    for (let index = 1; index < points.length; index++) {
      const length = (p) => Math.hypot(p[index].x - p[index - 1].x, p[index].y - p[index - 1].y);
      assert.ok(Math.abs(length(points) - length(rest)) < 1e-8);
    }
  }
  assert.ok(poseAt(450)[3].y < rest[3].y - 20);
  assert.ok(poseAt(1250)[3].y > rest[3].y + 20);
  assert.deepEqual(poseAt(1800), rest);
});

test('the 3D tail sweeps to both sides without stretching', () => {
  const rest = [
    { x: 0.56, y: 0.24, z: -0.14 }, { x: 0.84, y: 0.16, z: -0.18 },
    { x: 1.10, y: 0.24, z: -0.20 }, { x: 1.26, y: 0.42, z: -0.18 },
    { x: 1.36, y: 0.56, z: -0.16 }, { x: 1.48, y: 0.62, z: -0.14 },
  ];
  const swing = { elapsedMs: 450, durationMs: 1550, strength: 0.95, direction: 1 };
  const first = rest.map(point => ({ ...point }));
  const second = rest.map(point => ({ ...point }));
  bendTail(first, swing);
  bendTail(second, { ...swing, elapsedMs: 1250 });
  assert.deepEqual(first[0], rest[0]);
  assert.ok(first.at(-1).z < rest.at(-1).z - 0.3);
  assert.ok(second.at(-1).z > rest.at(-1).z + 0.3);
  for (const points of [first, second]) {
    for (let i = 1; i < points.length; i++) {
      const distance = (p) => Math.hypot(p[i].x - p[i - 1].x, p[i].y - p[i - 1].y, p[i].z - p[i - 1].z);
      assert.ok(Math.abs(distance(points) - distance(rest)) < 1e-8);
    }
  }
});


test('another pat lets the first tail swing finish before starting a new one', () => {
  const gesture = new PetGesture(() => 0);
  gesture.tap(0, 1);
  gesture.tap(850, -1);
  assert.equal(gesture.pose(900).tail.direction, 1);
  assert.equal(gesture.pose(900).tail.elapsedMs, 900);
  assert.equal(gesture.pose(1300).tail, null);
  gesture.tap(1700, -1);
  assert.equal(gesture.pose(1800).tail.direction, -1);
});


test('paw hit testing chooses the nearest paw within a small hit area', () => {
  const left = { x: 30, y: 55 };
  const right = { x: 45, y: 55 };
  assert.equal(pawSideAt({ x: 40, y: 57 }, left, right, 12), 'right');
  assert.equal(pawSideAt({ x: 30, y: 55 }, left, right, 12), 'left');
  assert.equal(pawSideAt({ x: 32, y: 75 }, left, right, 12), null);
});

test('a high five eases in and out and plays for the paw that was clicked', () => {
  const gesture = new PetGesture();
  gesture.highFive(100, 'right');
  assert.equal(gesture.pose(100).highFive, null);
  assert.ok(gesture.pose(360).highFive.amount > 0.95);
  gesture.highFive(400, 'left');
  assert.equal(gesture.pose(500).highFive.side, 'right');
  assert.equal(gesture.pose(900).highFive, null);
});

test('a sleeping cat can open its eyes before a queued pat begins', () => {
  const gesture = new PetGesture();
  gesture.tap(100, -1, 420);
  assert.equal(gesture.pose(400).tap, 0);
  assert.equal(gesture.pose(400).tail, null);
  assert.ok(gesture.pose(620).tap > 0);
  assert.ok(gentleWakeAmount(170) > 0.75);
  assert.equal(gentleWakeAmount(650), 0);
});

test('grumpy wake needs a burst of four actual presses', () => {
  assert.equal(isWakeBurst([100, 150, 200], 270), false);
  assert.equal(isWakeBurst([100, 230, 390, 550], 270), false);
  assert.equal(isWakeBurst([100, 150, 200, 240], 270), true);
});

test('an offered paw stays up, answers a tap and lowers if ignored', () => {
  const gesture = new PetGesture(() => 0);
  gesture.inviteHighFive(100, 'right', 1600);
  assert.equal(gesture.pose(1600).highFive, null);
  const offer = gesture.pose(2200).highFive;
  assert.equal(offer.side, 'right');
  assert.ok(offer.amount > 0.5 && offer.amount < 1);
  gesture.highFive(2200, 'right');
  assert.equal(gesture.pose(2200).highFive.amount, offer.amount);
  assert.ok(gesture.pose(2440).highFive.amount > offer.amount);
  assert.equal(gesture.pose(3700).highFive, null);

  gesture.inviteHighFive(4000, 'left');
  assert.ok(gesture.pose(4500).highFive);
  assert.equal(gesture.pose(8600).highFive, null);
});

test('fatigue starts neutral, settles with a slow exhale and releases before the paw rises', () => {
  assert.deepEqual(taskFatiguePose(0), { slump: 0, breath: 0, eyelids: 0 });
  assert.ok(taskFatiguePose(380).breath > 0.5);
  assert.ok(taskFatiguePose(900).slump > 0.8);
  assert.ok(taskFatiguePose(900).eyelids > 0.5);
  assert.deepEqual(taskFatiguePose(1600), { slump: 0, breath: 0, eyelids: 0 });
});
