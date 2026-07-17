# Preserve Gareji Safe Autopilot semantics

Gareji Board v0 uses stable work-item states, reconciliation rules, controller stops, fast exits, skip reasons, and queue caps through the `gareji_safe_autopilot_v0` policy profile. This preserves deterministic and evidence-first local operation, while future custom policy profiles may vary without silently changing the default safety behavior.
