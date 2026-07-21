# Style guide

The canonical style reference for SilkTerm. It covers prose, comments, naming, Rust conventions, formatting, and commit messages. When something here conflicts with a language's own well-established idioms or its enforced formatter, the idiom and the formatter win - see [Formatting](#formatting).

## Contents

- [Prose and documentation](#prose-and-documentation)
- [Comments](#comments)
- [File headers and licensing](#file-headers-and-licensing)
- [Naming](#naming)
- [Rust](#rust)
- [Formatting](#formatting)
- [Commit messages](#commit-messages)

## Prose and documentation

This applies to the README, design docs, backlog, this guide, and any other Markdown in the tree.

- Write for a human reader in a hurry. Short sentences. One idea each.
- Avoid run-on sentences that chain several ideas with dashes, semicolons, and parentheticals. Split them, or break them into nested bullets.
- Prefer nested bullet points over dense paragraphs when laying out several related points.
- Go easy on emphasis. Reserve bold, italics, and ALL-CAPS for the rare word that genuinely needs the weight.
- Skip flowery adjectives and adverbs. State what the thing does.
- ASCII only. Use `->` not an arrow glyph, `-` not an em or en dash. The one exception is `©` in copyright lines.
- Never hard-wrap. Treat a Markdown file like a word processor that wraps on its own: one paragraph or one bullet is one physical line. Use newlines, tabs, and spaces only for real structure - paragraph breaks, bullets, nesting, code blocks.
- Indent with tabs.
- Filenames are lowercase, except `README.md`.

## Comments

- Terse and human. Explain *why*, not *what* - the code already says what.
- No narration that restates the next line, and no banner dividers or decorative flowerboxing.
- ASCII only, same as prose above. Do not use Unicode in a comment unless you are documenting something that is itself about Unicode.
- Follow the surrounding file: match its comment density, its section-header style, and its idioms rather than introducing a new house style mid-file.
- Where a language has a well-established comment idiom (Rust doc comments, for example), that idiom overrides these preferences.

## File headers and licensing

- Every source file carries an SPDX identifier and a copyright line at the top:

	```rust
	// SPDX-License-Identifier: GPL-2.0-or-later
	// Copyright © 2026 Jim Collier
	```

- The project itself is licensed GPL-2.0-or-later.
- Standalone helper and utility scripts are usually MIT, regardless of the project license. Give those their own MIT header.

## Naming

- Use meaningful, human-searchable names. It should be easy to read and to search-and-replace `upperBound`; a bare `ub` is not.
- Do not overcorrect. A name does not need to be long or globally unique - it needs to be clear and easy to locate. Short conventional names are fine where they read cleanly.
- Single-letter loop counters and iterators (`for i in ...`) are fine when that is the idiomatic choice for the language.
- Follow the language's canonical case and word-order conventions (snake_case in Rust, and so on).

## Rust

Edition 2024. The guiding aim is code that is consistent within and across files: the same error strategy, the same naming, the same module layout throughout.

### Errors

- Errors are values. Return `Result<T, E>` and propagate with `?`.
- No `panic!`, `unwrap()`, or `expect()` outside tests, examples, or provably-unreachable cases. When a case really is unreachable, justify it with a short comment.
- Prefer `thiserror` for library-style error types and `anyhow` for application-level error handling.

### Ownership and borrowing

- Borrow first. Do not reach for `.clone()` to satisfy the borrow checker - restructure, borrow, or take a reference instead.
- If a clone is genuinely needed, add a comment saying why.
- Avoid gratuitous `Rc<RefCell<...>>`.
- Prefer `&str` over `String` and slices over `Vec` in arguments. Return owned types.

### Control flow

- Return early to keep the happy path at minimum indentation. Use guard clauses.
- `let ... else { return ... }` for "extract or bail".
- `?` to propagate instead of nesting `match` or `if let`.
- No `else` after a `return`.
- Collapse nested `if let` with `let`-else or, where it reads well, let-chains (`if let ... && ...`).
- Prefer flat combinators (`map`, `and_then`, `unwrap_or_else`) on `Option` and `Result` when they read cleanly. Fall back to `match` for genuine multi-arm logic.

### Types and abstraction

- Model mutually-exclusive states with enums and exhaustive `match`, not boolean flags. Avoid a catch-all `_` arm unless it is truly needed.
- Use the type system to make invalid states unrepresentable where it is cheap - newtypes, typestate.
- Traits and generics for abstraction; `dyn` only for heterogeneity. Compose; do not reach for inheritance-shaped designs.
- Derive rather than hand-roll (`Debug`, `Clone`, `PartialEq`, and so on). Derive `Debug` on all public types.

### Iterators

- Prefer iterator chains over manual loops while they stay readable.
- Break to a plain `for` loop when a chain would need more than about three combinators, or when clarity suffers.

### Documentation

- Document public items with `///`.
- Name things fully; no cryptic abbreviations.

## Formatting

- Rust is formatted by `rustfmt`. Run it and let its output win - do not hand-format against it.
- The project sets `rustfmt` to hard tabs at width four (`rustfmt.toml`). Tabs indent; spaces align. This is the one deliberate deviation from `rustfmt` defaults; everything else follows the defaults.
- Protect intentional hand-formatted data tables (a color matrix, a layout table) from reflow with `#[rustfmt::skip]` rather than fighting the tool.
- Code is expected to pass `clippy`. The build gate runs `clippy -D warnings`. Writing to the stricter `clippy::pedantic` bar is encouraged.
- Scripts with an enforced linter (Bash under `shellcheck`) must pass it.

## Commit messages

- Keep them brief and high-level - a short summary of what changed, the way you would jot it in a hurry.
- Put real detail in the issue, the pull request, or the code, not in a long enumerated commit body.
- No attribution trailers.
