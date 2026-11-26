# Contributing to dotori

Thanks for your interest in dotori. One person maintains it in their spare time, so replies can take a few days.

## Before you start

- To report a bug or ask for a feature, open an issue with one of the issue forms.
- To make a large change, open an issue first and describe the change. A short talk first saves you from work that does not fit the design.
- You can write issues and pull requests in English or Korean.

The design documents are in [`docs/`](../docs/README.md) and are written in Korean. Each document records the decisions for one part of dotori. If your change goes against a decision there, say so in the pull request and update the document in the same pull request.

## Set up

dotori uses [mise](https://mise.jdx.dev) to pin the Rust toolchain and the check tools.

1. Install mise.
2. On Linux, install the libraries that GPUI builds against.

	```bash
	sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev
	```

3. In the repository, install the pinned tools. This also turns on the Git hooks. The `commit-msg` hook checks the commit subject, and the `pre-push` hook runs every check.

	```bash
	mise install
	```

## Run the checks

```bash
mise run quick
```

This runs `rustfmt`, `clippy`, the tests, the `ast-grep` rules in `guards/`, and the `xtask` checks. It leaves out the window tests, which drive GPUI windows and take longer. Run them with `mise run window-test` when you change a window.

CI runs `mise run check` on Linux, macOS, and Windows. That is `mise run quick`, the window tests, and `cargo deny`. A pull request merges only when all three pass.

The checks enforce these rules, so you do not need to remember them:

- Code has no comments. Doc comments are only for public API contracts.
- Unit tests live in a sibling `<name>.test.rs` file.
- Files in the repository do not use en dashes, em dashes, middle dots, or CJK angle and corner brackets.
- Only the entries in `guards/root-entries.txt` can sit at the repository root.
- Each file in `docs/` follows the template in `docs/CLAUDE.md`. After you add or rename a document, run `mise run docs` to rebuild the index.

## Open a pull request

- Write the title as a [Conventional Commit](https://www.conventionalcommits.org), for example `fix(window): keep the scroll position after a reply`.
- Keep a pull request to one change. Several small pull requests are easier to review than one large one.
- Fill in the pull request template. Say how you checked the change. For a change to a window, add a screenshot.
- The maintainer squash-merges each pull request, so the title becomes the commit subject on `main`.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as in the [README](../README.md#license), without any additional terms or conditions.
