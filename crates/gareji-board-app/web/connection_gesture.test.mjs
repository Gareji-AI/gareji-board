import assert from "node:assert/strict";
import test from "node:test";

import { createConnectionGesture } from "./connection_gesture.js";

test("dragging an output port onto another node input commits one connection", () => {
  const previews = [];
  const commits = [];
  const gesture = createConnectionGesture({
    onPreview: (preview) => previews.push(preview),
    onCommit: (connection) => commits.push(connection),
  });

  gesture.start({ sourceId: "approach", pointerId: 7, point: { x: 120, y: 80 } });
  gesture.move({ pointerId: 7, point: { x: 260, y: 140 } });
  const committed = gesture.finish({ pointerId: 7, destinationId: "audit" });

  assert.equal(committed, true);
  assert.deepEqual(commits, [{ sourceId: "approach", destinationId: "audit" }]);
  assert.deepEqual(previews.at(-1), null);
});

test("dropping outside a destination or back on the source cancels safely", () => {
  const commits = [];
  const gesture = createConnectionGesture({ onCommit: (connection) => commits.push(connection) });

  gesture.start({ sourceId: "approach", pointerId: 3, point: { x: 10, y: 10 } });
  assert.equal(gesture.finish({ pointerId: 3, destinationId: null }), false);
  gesture.start({ sourceId: "approach", pointerId: 4, point: { x: 10, y: 10 } });
  assert.equal(gesture.finish({ pointerId: 4, destinationId: "approach" }), false);
  assert.deepEqual(commits, []);
});
