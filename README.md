# rat

`rat` is a small command-line tool for running the same shell command across
every subdirectory of a workspace, in parallel. Point it at a folder full of
repos or packages and it fans a command out to each of them at once (with
optional skip/only filters, timeouts, and a persisted default working
directory), instead of you cd-ing into each one by hand.

It's a Rust rewrite of an earlier C# tool (`ath`); this repo is the current,
actively developed version.

## Requirements

- [Rust](https://www.rust-lang.org/tools/install) 1.85+ (the project uses the
  2024 edition). Installing Rust via [rustup](https://rustup.rs/) also gives
  you `cargo`, which is all you need below.

## Build from source

```bash
git clone https://github.com/p-w-g/rat.git
cd rat
cargo build --release
```

The compiled binary is written to `target/release/rat` (`rat.exe` on
Windows).

## Install

The simplest option is to let Cargo build and install it into `~/.cargo/bin`
(already on your `PATH` if you installed Rust via rustup):

```bash
cargo install --path .
```

Then, from any terminal:

```bash
rat help
```

Prefer to manage the binary yourself instead? Copy it wherever you keep
personal tools and make sure that location is on your `PATH`:

```bash
# after `cargo build --release`
cp target/release/rat /usr/local/bin/        # macOS/Linux example
```

On Windows, copy `target\release\rat.exe` to a folder that's on your `PATH`
(or add `target\release` to `PATH` directly).

## Usage

`rat help` always prints the current, in-the-box copy of everything below.
There are three top-level commands:

```
rat help            # this help text
rat fep <command>   # run <command> in every subfolder of the working folder
rat cfg <subcommand> # read/edit rat's own config
```

### `fep`: run a command across every subfolder

```
rat fep <command...> [flags]
```

`<command...>` is whatever you want run, exactly as you'd type it on the
command line - e.g. `rat fep git pull`, `rat fep npm install`. It's run
through your platform's shell (`cmd.exe` on Windows, `/bin/bash` elsewhere)
once per immediate subdirectory of the working folder, in parallel.

`[flags]` are **rat's own** flags, not your command's. All optional:

| flag                  | meaning                                                                                          |
| --------------------- | ------------------------------------------------------------------------------------------------- |
| `--local`              | use the current directory for this run, even if a default folder is set via `cfg here`            |
| `--only-a-b-c`         | only run in subfolders whose path contains `a`, `b`, or `c`                                        |
| `--skip-a-b-c`         | skip subfolders whose path contains `a`, `b`, or `c` (ignored entirely if `--only` is also given)  |
| `--sustain`            | wait as long as it takes, ignoring any timeout                                                     |
| `--timeout-30`         | timeout *this run* after 30 seconds, overriding the configured timeout                             |

> `--timeout-0` is a **0-second timeout**, not "disabled" - that's different
> from `cfg to 0`, which does disable it. Use `--sustain` or `cfg nto` if you
> want no timeout.

### The #1 gotcha: your command's flags vs. rat's flags

rat parses `--flags` out of the command you pass to `fep` *before* your
command ever runs - your shell doesn't get a say. Specifically:

- Any `--word...` in your command where `word` is one of rat's reserved
  words (`local`, `skip`, `only`, `sustain`, `timeout`) is captured by rat
  as its own flag and **never reaches your command**.
- Any other unrecognized `--flag` is **silently dropped** - not passed to
  your command, not treated as a rat option, just gone.
- Single-dash flags (`-m`, `-rf`, `-n`, ...) are **never** touched by rat and
  always reach your command untouched.

| you type                                        | what happens                                                                 |
| ------------------------------------------------ | ----------------------------------------------------------------------------- |
| `rat fep rm -rf node_modules`                     | safe - `-rf` is single-dash, passed straight through                          |
| `rat fep git commit -m "this fails with ath"`     | safe - `-m` is single-dash                                                    |
| `rat fep git merge --skip-commit`                 | **not safe** - rat reads `--skip-commit` as its own `--skip commit` filter; git runs as plain `git merge`, silently missing `--skip-commit` |

If you need to run something with a `--flag` that collides with rat's
reserved words, put the real invocation in a small script and call the
script instead (`rat fep ./do-the-thing.sh`), so nothing reaches rat's
parser except the script name.

### `cfg`: configuration

Config lives at `~/.ratconfig` (JSON), created on first use of `cfg`.

| subcommand                          | meaning                                                                 |
| ------------------------------------ | ------------------------------------------------------------------------ |
| `cfg path`                           | print the config file's path                                            |
| `cfg file`                           | print the config file's contents                                        |
| `cfg here`                           | set the current directory as the default working folder for `fep`      |
| `cfg away`                           | unset the default working folder (back to using CWD)                    |
| `cfg ignore <folders...>`            | permanently ignore these folders in every `fep` run, e.g. `cfg ignore .git .idea` |
| `cfg heed <folders...>` / `--all`    | stop ignoring these folders, or clear the whole ignore list with `--all` |
| `cfg to <seconds>`                   | set a default timeout for `fep`; `0` disables it                        |
| `cfg nto`                            | disable the default timeout                                             |

## Development

```bash
cargo test      # run the test suite
cargo build     # debug build
```

## Roadmap

- Packaged release binaries (GitHub Releases) for macOS/Linux/Windows.
- An npm wrapper package so JS/TS developers can `npm install -g` it without
  a local Rust toolchain.
