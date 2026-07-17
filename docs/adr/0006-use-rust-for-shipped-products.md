# ADR 0006: Use Rust for shipped Gareji products

Status: accepted.

Gareji Core, Runner, MCP, Board state, Knowledge Adapters, and shipped validation commands use Rust. Board uses Dioxus Desktop so application logic remains Rust while HTML and CSS provide an accessible portfolio interface. Gareji does not add Tauri around Dioxus because the second desktop host would add an Interface without providing a second required Adapter.

Python is not a product runtime dependency. The original fixture validator is replaced by a Rust `xtask` in a separate mechanical commit. JavaScript application logic is excluded by default; a future WebView limitation may justify a small, isolated Adapter after its need is demonstrated.

The repository keeps separate domain, SQLite store, local Bridge, and desktop application crates. The Store Module owns schema creation and queries behind its Rust Interface; the UI and Bridge never issue SQL. Cross-product behavior uses narrow versioned Bridge Interfaces rather than direct access to another product's database.
