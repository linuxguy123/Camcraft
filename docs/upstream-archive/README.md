# Upstream GitHub records

This branch preserves a public-API snapshot of issues and pull requests from
`NeoTech/Camcraft` after that repository became public again. It is an archive,
not a live mirror and not a replacement for the original discussion threads.

- [2026-11-09 archive](NeoTech-Camcraft-20261109/README.md)
- [2026-11-09 update: 12 new issues and 36 new pull requests](NeoTech-Camcraft-20261109-175838/README.md)
- [Issue index](NeoTech-Camcraft-20261109/issues.csv)
- [Pull request index](NeoTech-Camcraft-20261109/pulls.csv)
- [Capture manifest](NeoTech-Camcraft-20261109/manifest.json)

Run `scripts/archive_github_metadata.py OWNER/REPO OUTPUT_DIR` with an
authenticated GitHub CLI to capture a later snapshot. Use a new output
directory for each run so changes to upstream state remain visible.
Pass `--baseline` once per earlier archive to export only records not
already saved; earlier issue and PR discussions remain in those archives.
The fork's `Preserve upstream` workflow checks every 15 minutes and uses
`--baseline-root --include-updates` to keep later edits and comments too.
Run it manually from Actions whenever an immediate capture is needed.
