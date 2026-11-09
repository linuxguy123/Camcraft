# CamCraft multi-setup preview ({{TAG}})

This is a community test build from [the multi-setup branch]({{REPO_URL}}/tree/{{TAG}}), based on CamCraft 0.14.0. It is separate from the upstream releases and later upstream setup changes. Source commit: [`{{SHA}}`]({{REPO_URL}}/commit/{{SHA}}).

## What to test

- Create a new project with Setup1, add a second setup, and switch between them.
- Give each setup its own Toolpath CS orientation and one Machine CS register (G54-G59).
- Generate and post operations in each setup; inspect the toolpaths and simulate one setup at a time.
- Check that the shared model and stock follow the active setup's Toolpath CS.
- Try the Toolpath CS origin markers at corners, edge midpoints, planar centers, and circular centers.

## Known limits

- The machined stock left by one setup is not yet carried into the next setup.
- Workholders is a placeholder.
- Start with a new project; migration from later upstream project formats is not supported.
- The binaries and package metadata still identify as version 0.14.0. The preview tag and download filenames distinguish these builds from the upstream 0.14.0 release.
- If CamCraft 0.14.0 is already installed, its package manager may consider the preview DEB/RPM the same version. The portable ZIP or AppImage avoids that version collision.

Please report results and example projects on [the fork's issue tracker]({{REPO_URL}}/issues). Inspect generated NC before using it on a machine.
