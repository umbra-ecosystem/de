Fixtures for the `bkt` adapter tests.

DERIVED FROM DOCUMENTATION AND SOURCE, NOT CAPTURED FROM A REAL INSTALL.

Shapes follow the Go structs and json tags of avivsinai/bitbucket-cli (master, v0.32.1:
pkg/bbcloud/*.go, pkg/cmd/pr/*.go, pkg/cmd/auth/auth.go) for what `bkt ... --json` prints,
and the Bitbucket Cloud REST 2.0 documentation for what `bkt api` passes through
(pipelines, steps, environments, comments). Values are invented. Fields marked UNVERIFIED in
`bkt.rs` (notably the deployment `environment` of a pipeline step) are a best reading of the
REST docs; replace these files with real output from the probe when available.

Exception: auth_status_null.json is the real structure of `bkt auth status --json` on a
machine that is not logged in (no personal data). The other files remain source-derived
because no bkt login was available.
