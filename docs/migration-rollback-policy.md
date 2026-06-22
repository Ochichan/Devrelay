# Migration Rollback Policy

DevRelay metadata migrations are forward-only and run inside a SQLite
transaction. A failed migration must leave the database at the previous applied
version.

## Rules

- Every migration has a monotonically increasing version.
- Applied versions are recorded in `schema_migrations`.
- Migration SQL must be safe to run once and skipped when already applied.
- Rollback means restoring from the previous database file or a pinned snapshot,
  not running destructive down migrations.
- Before a future destructive migration, create a backup copy and document the
  recovery path in release notes.

