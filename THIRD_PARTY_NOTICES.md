# Third-party notices

hydroid links the following and ships them inside its binary:

- **ruff / ty crates** (`ruff_db`, `ruff_python_ast`, `ty_python_semantic`, …) — MIT,
  Copyright (c) 2022 Charles Marsh. <https://github.com/astral-sh/ruff>
- **typeshed** stub files, bundled by `ty_vendored` — Apache License 2.0,
  Copyright the typeshed contributors. <https://github.com/python/typeshed>
  Full license text: <https://github.com/python/typeshed/blob/main/LICENSE>
- Other Rust dependencies are listed in `Cargo.lock`; their licenses (MIT and/or Apache-2.0)
  can be listed with `cargo license`.
