export async function runAgentLoopWithFeedback({
  workItemId,
  now = Date.now,
  update,
  waitForPaint,
  start,
  reload,
}) {
  update({ workItemId, startedAt: now() });
  try {
    await waitForPaint();
    const result = await start({ workItemId });
    await reload();
    return result;
  } finally {
    update(null);
  }
}

export function agentLoopFeedback(activeRun, now = Date.now()) {
  if (!activeRun) return null;
  const elapsedSeconds = Math.max(0, Math.floor((now - activeRun.startedAt) / 1_000));
  const minutes = Math.floor(elapsedSeconds / 60);
  const seconds = String(elapsedSeconds % 60).padStart(2, "0");
  const elapsed = `${minutes}:${seconds}`;
  return {
    buttonLabel: elapsedSeconds === 0 ? "Starting Agent Loop…" : "Agent Loop running…",
    status: `Codex is working in an isolated Git worktree. Elapsed ${elapsed}. Keep this window open.`,
  };
}
