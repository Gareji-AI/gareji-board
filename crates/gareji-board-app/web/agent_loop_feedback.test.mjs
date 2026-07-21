import assert from "node:assert/strict";
import test from "node:test";

import { runAgentLoopWithFeedback } from "./agent_loop_feedback.js";

test("publishes visible feedback before starting a slow Agent Loop and refreshes after completion", async () => {
  const events = [];
  let finishRun;
  let markStarted;
  const runFinished = new Promise((resolve) => { finishRun = resolve; });
  const runStarted = new Promise((resolve) => { markStarted = resolve; });

  const execution = runAgentLoopWithFeedback({
    workItemId: "CORE-2",
    now: () => 1_000,
    update: (activeRun) => events.push(["update", activeRun]),
    waitForPaint: async () => events.push(["paint"]),
    start: (request) => {
      events.push(["start", request]);
      markStarted();
      return runFinished;
    },
    reload: async () => events.push(["reload"]),
  });

  assert.deepEqual(events[0], ["update", { workItemId: "CORE-2", startedAt: 1_000 }]);
  await runStarted;
  assert.deepEqual(events.slice(0, 3), [
    ["update", { workItemId: "CORE-2", startedAt: 1_000 }],
    ["paint"],
    ["start", { workItemId: "CORE-2" }],
  ]);

  finishRun({ message: "Run succeeded." });
  const result = await execution;

  assert.deepEqual(result, { message: "Run succeeded." });
  assert.deepEqual(events.slice(-2), [["reload"], ["update", null]]);
});

test("clears visible feedback when Agent Loop startup fails", async () => {
  const updates = [];

  await assert.rejects(
    runAgentLoopWithFeedback({
      workItemId: "CORE-2",
      now: () => 2_000,
      update: (activeRun) => updates.push(activeRun),
      waitForPaint: async () => {},
      start: async () => { throw new Error("unavailable"); },
      reload: async () => assert.fail("reload must not run after a failed start"),
    }),
    /unavailable/,
  );

  assert.deepEqual(updates, [
    { workItemId: "CORE-2", startedAt: 2_000 },
    null,
  ]);
});
