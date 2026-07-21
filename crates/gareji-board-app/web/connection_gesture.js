export function createConnectionGesture({ onPreview = () => {}, onCommit = () => {} } = {}) {
  let active = null;

  function clear() {
    active = null;
    onPreview(null);
  }

  return {
    start({ sourceId, pointerId, point }) {
      active = { sourceId, pointerId };
      onPreview({ sourceId, point });
    },

    move({ pointerId, point }) {
      if (!active || active.pointerId !== pointerId) return false;
      onPreview({ sourceId: active.sourceId, point });
      return true;
    },

    finish({ pointerId, destinationId }) {
      if (!active || active.pointerId !== pointerId) return false;
      const { sourceId } = active;
      const canCommit = Boolean(destinationId) && destinationId !== sourceId;
      clear();
      if (canCommit) onCommit({ sourceId, destinationId });
      return canCommit;
    },

    cancel(pointerId) {
      if (!active || active.pointerId !== pointerId) return false;
      clear();
      return true;
    },
  };
}
