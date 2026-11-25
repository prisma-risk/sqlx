# PrismaRisk SQLx fork

This fork adds TCP keepalive and `TCP_USER_TIMEOUT` options for PostgreSQL connections. Commit `70c7892e` adds these options to `sqlx-core` and `sqlx-postgres`.

The `main` branch is the PrismaRisk release line. Its upstream base is `v0.9.0` at `75bc0487`. The PrismaRisk commits follow that base.

Platform uses `.github/workflows/release.yml` to publish `sqlx`, `sqlx-core`, `sqlx-macros`, `sqlx-macros-core`, `sqlx-mysql`, `sqlx-postgres`, and `sqlx-sqlite`. These crates publish only to `prismarisk`. Publication to crates.io is prohibited.

Use the Kellnr crates and keep each git `[patch.crates-io]` at or before `70c7892e` because Cargo ignores a `0.0.0-dev` patch for `0.9.0`.

The fork uses Rust 1.100.0 to match the PrismaRisk release runner image.

Kellnr versions are immutable. Release the next fork change as `0.9.1` or later. Record the upstream base in this file.

Fetch the later tag from `launchbadge/sqlx`. Create a topic branch from the PrismaRisk release line. Rebase the PrismaRisk commits with `git rebase --onto <later-upstream-tag> 75bc0487`. Resolve conflicts in the fork commits. Update the upstream tag and commit in this file. Run the workspace checks before you propose the result for review.
