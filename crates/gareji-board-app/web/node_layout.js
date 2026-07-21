const CANVAS_MARGIN = 12;
const MAX_LAYOUT_ID_LENGTH = 64;

export function canvasLayoutId(prefix, stableId) {
  const raw = `${prefix}-${stableId}`;
  if (raw.length <= MAX_LAYOUT_ID_LENGTH) return raw;

  let hash = 0xcbf29ce484222325n;
  for (const byte of new TextEncoder().encode(raw)) {
    hash = BigInt.asUintN(64, (hash ^ BigInt(byte)) * 0x100000001b3n);
  }
  const suffix = `-${hash.toString(16).padStart(16, "0")}`;
  const stem = raw.slice(0, MAX_LAYOUT_ID_LENGTH - suffix.length).replace(/[-_]+$/, "");
  return `${stem}${suffix}`;
}

export function positionsFromLayout(nodes, layout) {
  const nodeIds = new Set(nodes.map((node) => node.id));
  const positions = new Map(
    (layout?.positions || [])
      .filter((item) => nodeIds.has(item.node_id))
      .map((item) => [item.node_id, { x: item.x, y: item.y }]),
  );
  nodes.forEach((node, index) => {
    if (!positions.has(node.id)) {
      positions.set(node.id, {
        x: 46 + (index % 4) * 230,
        y: 54 + Math.floor(index / 4) * 170,
      });
    }
  });
  return positions;
}

export function moveNode(start, origin, current, bounds) {
  const maxX = Math.max(CANVAS_MARGIN, bounds.canvasWidth - bounds.nodeWidth - CANVAS_MARGIN);
  const maxY = Math.max(CANVAS_MARGIN, bounds.canvasHeight - bounds.nodeHeight - CANVAS_MARGIN);
  return {
    x: Math.min(maxX, Math.max(CANVAS_MARGIN, start.x + current.x - origin.x)),
    y: Math.min(maxY, Math.max(CANVAS_MARGIN, start.y + current.y - origin.y)),
  };
}

export function serializePositions(layoutId, positions) {
  return {
    graph_id: layoutId,
    positions: [...positions]
      .map(([node_id, point]) => ({ node_id, x: point.x, y: point.y }))
      .sort((left, right) => left.node_id.localeCompare(right.node_id)),
  };
}
