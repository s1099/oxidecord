# Code debt

Gaps left in features that already shipped: shortcuts, unhandled edge cases, known bugs,
hacks and TODOs. Entries are grouped by feature. Add one when work leaves a gap behind, and
delete it once the gap is closed. Features the app doesn't have at all go in
[`MISSING_FEATURES.md`](MISSING_FEATURES.md).

Entry format:

```md
## <Feature name>

- **<Short gap title>** — what's missing or wrong, and why it was left. `src/path.rs:123`
```
