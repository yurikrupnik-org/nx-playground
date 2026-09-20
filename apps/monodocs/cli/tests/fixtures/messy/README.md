# messy

Fixture workspace for the `monodocs lint` rules. Its root document is deliberately clean, so any
finding a test sees comes from the project below it.

- `kcl-broken` — one document that trips every rule exactly once.
- `undocumented` — a package with a manifest and no `README.md`.
