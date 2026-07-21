# Gareji Board interface design

Gareji Board is a Tauri operations desk, not an AI persona. Its interface should make work, control boundaries, evidence, and the next human decision legible without presenting autonomy as magic.

## Principles

- **Show control, not intelligence theater.** Name actions by their effect. Safe Autopilot is previewed before it can start work, and approval stages remain explicit human decisions.
- **Keep the work surface in view.** At desktop sizes the application shell, navigation, and workspace header stay fixed. A Kanban lane, project list, graph canvas, or detail pane owns its own overflow instead of making the whole document scroll.
- **Use operational hierarchy.** Overview answers what is moving, which projects have capacity, and what action is available. Work, Projects, Automation, and Activity are persistent destinations rather than decorative cards.
- **Treat graphs as first-class workspaces.** Control Graph uses a project master rail and a graph detail pane. Blueprint Studio keeps notes, the node canvas, and the inspector visible together so connections are discoverable.
- **Use restrained visual language.** System sans-serif type, flat dark surfaces, compact spacing, small radii, and the existing deep-green trust palette replace editorial type, decorative gradients, large shadows, and generic purple AI styling.
- **Reserve color for meaning.** Green communicates trusted or available control paths; warning and blocked colors describe state. Labels and structure carry the same meaning so color is never the only signal.
- **Expose details progressively.** Human-readable names come first. Local paths and storage details remain available through titles, inspectors, and disclosures instead of dominating the primary surface.

## Desktop layout contract

The supported desktop window starts at 1024 pixels wide. From 1024 × 650 upward, the outer document does not scroll. Navigation and workspace identity remain stable while the active work surface controls overflow. Below that height, ordinary document scrolling is retained so content is never clipped.

The local presentation is embedded in the Tauri binary. It does not load a remote website or require a frontend server. Tauri commands expose one bounded desktop read model and explicit actions; paths, SQL, Runner construction, and policy stay on the Rust side of the seam.

Kanban lanes scroll vertically and the board scrolls horizontally. Control Graph keeps the project rail fixed beside a scrollable detail pane. Blueprint Studio uses a three-pane authoring layout and collapses responsively when width is constrained.

## Interaction and accessibility

Interactive controls use visible focus states and stable hover/pressed states without layout-shifting motion. Motion is subtle and disabled when reduced motion is requested. Disabled actions remain visibly disabled, error and status text stay explicit, and native buttons, labels, and headings preserve keyboard and accessibility-tree semantics.
