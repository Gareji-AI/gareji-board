# ADR 0016: Use Tauri for the Board desktop host

Status: accepted. Supersedes the desktop-host decision in ADR 0006.

Gareji Board uses Tauri 2 as its shipped desktop host. The presentation is a small set of local HTML, CSS, and JavaScript modules embedded into the binary; product behavior remains in Rust.

The seam is one deep Board desktop Module. Its Interface returns a bounded desktop read model and accepts explicit human intents for Work item transitions, project creation, Safe Autopilot preview and start, Graph layout, immutable Graph and Blueprint publication, and Project graph binding. Tauri commands are thin transport adapters to this Interface. They do not issue SQL or reinterpret Board domain rules.

This replaces Dioxus Desktop rather than adding a second desktop host around it. The Dioxus dependency and legacy UI implementation are removed. Gareji Board continues to use the operating system WebView and keeps Board domain, Store, Core bridge, and Runner behavior in their existing Rust Modules.

The presentation layer may use JavaScript only for view state, accessible interaction, node dragging, and Tauri invocation. SQL, filesystem discovery, Runner construction, trust decisions, Safe Autopilot selection, and graph validation remain Rust responsibilities. No remote web origin is loaded, and the content security policy permits only embedded application assets and Tauri IPC.

Consequences:

- The existing `cargo run -p gareji-board-app` and demo launcher continue to open the desktop application without a Node.js runtime or frontend development server.
- Desktop distribution can use Tauri's normal packaging path later without another host migration.
- The UI and native behavior now cross an explicit serialization seam, so new presentation data must be added deliberately to the bounded desktop read model.
- Rust tests exercise the desktop Module Interface; browser-side syntax and real-window interaction are verified separately.
