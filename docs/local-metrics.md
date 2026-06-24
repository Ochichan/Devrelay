# Local Metrics

Last updated: 2026-06-24

DevRelay metrics are local by default. `metrics.export` and
`devrelay metrics export` read local metadata only and write a JSON report under
`$DEVRELAY_HOME/metrics/` unless `--out` is provided.

The export is redacted by default and records:

- verified continuation attempts and successes from handoff records and journal
  phases
- checkpoint successes from stored snapshot metadata
- checkpoint failure reasons from non-success `snapshot.published` audit events
- apply verification failures when non-success `snapshot.applied` audit events
  exist
- handoff phase and committed total durations from handoff journal timestamps
- scheduler choice reasons from task-run metadata, including the canonical
  `scheduler_choice_reason` produced from scheduler selection output
- hydration state counts and hydrate duration samples from persisted hydration
  records

The report does not include source code, snapshot objects, or raw logs. It
includes `privacy.local_by_default: true`, `source_code_included: false`, and
`snapshot_objects_included: false` so support workflows can verify the export
boundary mechanically.

Known recording gaps are explicit in `recording_gaps`. Legacy hydration records
that predate duration fields do not produce duration samples until the workspace
hydrates again.

Scheduler metrics expect task-run metadata to preserve the scheduler selection
reason. New task runners should use the core `scheduler_selection_metadata`
helper so exports can group runs by stable reasons such as
`highest-eligible-score`, `no-eligible-target`, and `no-candidates` without
parsing free-form explanation text.
