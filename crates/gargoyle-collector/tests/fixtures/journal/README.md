# Journal fixtures

`journalctl -o json` output for timer-service runs, as `LinuxCollector::timer_history` requests it
(see `src/linux/journal.rs` for the exact arguments and the meaning of each `MESSAGE_ID`).

| File | Source | Shows |
|---|---|---|
| `man-db.json` | real (Arch, systemd 262) | `Type=oneshot` successes: Starting → Deactivated → Finished |
| `shadow.json` | real | `Type=simple` successes: Started → Deactivated, no "Starting" |
| `pcrproduct-failed.json` | real | `exit-code` failures with `EXIT_STATUS=1`; first run's start outside the window |
| `pcrproduct-start-limit.json` | real | `start-limit-hit` logged under the previous run's invocation |
| `tmpfiles-clean-cut.json` | real, first line removed | oneshot whose "Starting" is before `--since` |
| `handmade.json` | hand-made | timeout (signal), core dump, forged non-PID-1 entries, other unit, malformed line, missing timestamp, binary field, dependency failure, run in progress |

Real captures were taken read-only with the collector's field list (before `COMMAND` was added). `__CURSOR`, `_BOOT_ID`,
`__SEQNUM(_ID)` and `__MONOTONIC_TIMESTAMP` were removed; no `MESSAGE`, hostname or user fields are
requested, so nothing else needed scrubbing.
