# Use SQLite for local control state

Gareji stores local coordination and operational state in SQLite under the operating system's application-data location. Board-owned records and Core-owned active-work references, checkpoint ledger, outbox, and delivery receipts remain behind separate Module Interfaces even if they share one database; SQLite provides durable offline capture and per-destination retries while JSON remains limited to fixtures, import/export, and the demo Knowledge Adapter.
