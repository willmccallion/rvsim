# Changelog — v2.0.1

Released: 2026-10-07

A front-end timing fix: decode no longer refetches a path fetch already
took, which made IPC depend on unrelated parameters.

## Fixed

- Decode refetched the path fetch had already taken whenever it found a
  branch the BTB missed, predicted it not taken, and fetch had predicted a
  younger branch: it undid the younger predictions, which were made
  without the branch in the histories, and redirected fetch to remake
  them. A never-taken branch, which never enters the BTB, paid this every
  loop iteration, so how often it happened depended on front-end timing
  and moved IPC with unrelated parameters (#143). Decode now redoes those
  predictions in program order as it reaches them and redirects only when
  the path changes. `bp.decode_redirects` falls by up to 99% and the
  affected programs take up to 10% fewer cycles; mispredictions are
  unchanged.
