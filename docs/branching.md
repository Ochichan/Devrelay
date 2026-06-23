# Branch Naming

Use short, milestone-oriented branch names:

```text
<type>/<milestone>-<short-topic>
```

Allowed types:

- `feat` for new product behavior
- `fix` for defects
- `test` for test-only changes
- `docs` for documentation-only changes
- `chore` for repository and build maintenance

Examples:

```text
feat/m0-status-parser
test/m0-round-trip-fixtures
docs/global-security-policy
chore/global-ci
docs/m7-ui-slice
test/safety-no-silent-overwrite
```

Keep each branch scoped to one checklist item or one tightly related group of
items.
