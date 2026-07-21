# Gareji Board demo Execution workspace

This fixture contains the versioned Agent instructions and Skills used by the disposable Gareji Board sample. It represents an Execution workspace and is intentionally separate from the Markdown Knowledge workspace under `examples/demo-workspace`.

The Agent catalog may inspect these files without reading their contents into Board state. Bundled Skills are enabled only for the disposable sample; copying this directory does not enable or trust them for another project.

Demo-only acceptance criteria live under `work-items/`. Agent Loop runs may change only their isolated Git worktree; the fixture and the connected demo checkout remain unchanged.
