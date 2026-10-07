# @nyaterm/plugin-cli

Standalone plugin authoring tools. The npm package bundles platform binaries, browser SDK and templates; it does not clone NyaTerm or compile Rust to pack/inspect UI plugins. Node 22+ is required. Native plugin development additionally requires Rust 1.94+.

```sh
npm install -g @nyaterm/plugin-cli
nyaterm-plugin create example.hello ./hello
nyaterm-plugin pack ./hello ./hello-1.0.0.nyap
nyaterm-plugin inspect ./hello-1.0.0.nyap
nyaterm-plugin create example.native ./native --template rust
```

Pack/inspect use the same Rust validator as the desktop installer. Packages are unsigned author candidates. Submit them through the Store review/signing process; never use an official signing key in an author repository. Rust templates consume the SDK through a pinned public Git commit.

For local packaging from NyaTerm source:

```sh
cargo build --release --manifest-path src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml --bin nyaterm-plugin
npm pack ./plugins/cli
```

A local tarball contains the current platform binary. The **Build plugin CLI** workflow assembles all six supported binaries, checks the release completeness and produces a publishable npm tarball. Its optional publication job requires the `plugin-cli-publishing` environment and `NPM_TOKEN`; build-only runs do not publish. Use a reviewed platform commit and verify npm package name ownership before first publication.
