# Contributing to GhostAlpha

Thanks for your interest in improving GhostAlpha. Contributions are welcome through pull requests.

## How changes get in

1. Fork the repository and create a branch from `main`.
2. Make your change, with tests (see below).
3. Open a pull request against `main` describing **what** changed and **why**.
4. CI must pass and the maintainer must approve the pull request before it can be merged. Pushing new commits after approval requires a fresh review.

Nobody pushes directly to `main`. For larger changes (new commands, new data sources, changes to the projection model), please open an issue first so we can agree on the approach before you invest time in it.

## Before you open a pull request

Run the same checks CI runs:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

- **Tests:** new behaviour needs tests; bug fixes need a regression test that fails without the fix.
- **Tests are the contract:** don't weaken or delete an existing assertion to make a change pass. If you believe a test is wrong, say so in the pull request and explain why.
- **Scope:** keep pull requests focused. Unrelated refactors or formatting changes make review harder; send them separately.

## Never commit personal financial data

GhostAlpha works with brokerage exports and vesting schedules. Keep yours out of the repository:

- Put your own files in `my_data/` (create templates with `ghost-alpha init-data my_data`). It is gitignored, as are common broker-export filenames.
- Test fixtures must be **synthetic**: made-up account numbers, quantities, prices and cost basis. See `tests/fixtures/` for examples.
- Never commit API keys or a real `config.toml`; use `config.example.toml` as the reference.

Pull requests that contain real account data, API keys or other personal information will be closed. If that happens, rotate any exposed keys immediately: closing the pull request does not remove the data from your fork or from GitHub's caches.

## Reporting bugs

Open an issue with the command you ran, what you expected, what happened, and your OS and `rustc --version`. Replace any real positions with made-up numbers before pasting output.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE) that covers this project.
