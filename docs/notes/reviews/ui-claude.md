# UI Specification Review: Privacy Gate v1

Reviewer: Claude (via `claude -p`; keychain auth). Date: 2026-08-23.

(Raw Claude output from the shared review prompt. Claude did not have parent spec bodies
in-session; D-1 asked for an api.md cross-check, which the author did.)

## J. Top 5 (as received)

1. Resolve §6 dual ordering into one sequence (file picker vs `set_retention_default`).
2. Add a first-paint test row to §16 (C-TEST-8 names first paint).
3. Scope §13.1 report-save explicitly under C-ARCH-2, or extend architecture.md.
4. Specify approval first-paint above 200 fields.
5. Cross-check non-packet commands against api.md (not invented).

Other notes: C-1 save exception widened to integrity JSON; G-1 two legal import orders;
E-1 first-paint untested; FR-6.1/6.2 warning-not-toast praised.
