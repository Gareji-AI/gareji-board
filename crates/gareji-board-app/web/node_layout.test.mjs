import assert from "node:assert/strict";
import test from "node:test";

import {
  canvasLayoutId,
  moveNode,
  positionsFromLayout,
  serializePositions,
} from "./node_layout.js";

test("a Blueprint node follows the pointer and remains inside the canvas", () => {
  assert.deepEqual(
    moveNode({ x: 100, y: 80 }, { x: 40, y: 30 }, { x: 175, y: 125 }, {
      canvasWidth: 700,
      canvasHeight: 500,
      nodeWidth: 180,
      nodeHeight: 104,
    }),
    { x: 235, y: 175 },
  );

  assert.deepEqual(
    moveNode({ x: 600, y: 450 }, { x: 0, y: 0 }, { x: 400, y: 300 }, {
      canvasWidth: 700,
      canvasHeight: 500,
      nodeWidth: 180,
      nodeHeight: 104,
    }),
    { x: 508, y: 384 },
  );
});

test("Blueprint layouts use the stable legacy-compatible identity", () => {
  assert.equal(canvasLayoutId("blueprint", "release-loop"), "blueprint-release-loop");
  const longId = canvasLayoutId("blueprint", "a".repeat(64));
  assert.equal(longId, "blueprint-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-c555c6554bee3017");
  assert.equal(longId.length, 64);
});

test("saved positions are restored and serialized deterministically", () => {
  const blueprint = { nodes: [{ id: "audit" }, { id: "approach" }, { id: "finish" }] };
  const positions = positionsFromLayout(
    blueprint.nodes,
    { positions: [
      { node_id: "finish", x: 420, y: 210 },
      { node_id: "removed-node", x: 900, y: 500 },
    ] },
  );

  assert.deepEqual(positions.get("finish"), { x: 420, y: 210 });
  assert.deepEqual(positions.get("audit"), { x: 46, y: 54 });
  assert.equal(positions.has("removed-node"), false);
  assert.deepEqual(serializePositions("blueprint-release-loop", positions), {
    graph_id: "blueprint-release-loop",
    positions: [
      { node_id: "approach", x: 276, y: 54 },
      { node_id: "audit", x: 46, y: 54 },
      { node_id: "finish", x: 420, y: 210 },
    ],
  });
});
