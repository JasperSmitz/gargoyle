# Liner report fixtures

Used by `crates/gargoyle-core/tests/liner.rs` (ADR 0007, ROADMAP M7.1).

- `golden-report.json`: Liner's own golden report, copied **unchanged** from
  `tests/golden/report.json` of the Liner repository at commit `276df28` (the file last changed
  in `260bba9`). Schema `1.0.0`. It is synthetic (Liner's test library, `root` is
  `/library`) and holds no real library data. Replace it only with a newer copy of that file,
  and update this note.

The others are synthetic variants of it, written for Gargoyle's tests. Nothing here comes from a
real library report.

- `schema-1.7.0-unknown-fields.json`: a later 1.x report with unknown fields at every level, an
  unknown issue code, error kind, audio format and tag type. Must parse.
- `schema-2.0.0.json`: a major-2 report with a different shape. Must be refused as unsupported.
- `missing-schema-version.json`: no `schema_version`. Invalid.
- `bad-generated-at.json`: `generated_at` that is not RFC 3339. Invalid.
- `unknown-severity.json`: an issue with severity `fatal`, outside Liner's closed enum. Invalid.

The cap cases (more than 500 issues, more than 50 paths per issue, more than 100 errors) are
built from the golden report in the test itself rather than stored as large files.
