# PHPantom — Configuration

Per-project configuration file for user preferences and optional features like diagnostic proxying.

## File

- **Name:** `.phpantom.toml`
- **Location:** Project root (next to `composer.json`).
- **Format:** TOML. Human-readable, supports comments, native Rust support via the `toml` crate.
- **Version control:** Up to each developer. The dot-prefix signals personal tooling config. Developers can gitignore it globally or per-project. PHPantom should never assume it is committed.

## Schema

```toml
# .phpantom.toml

[php]
# Override the detected PHP version.
# When unset, PHPantom infers from composer.json's platform or require.php.
# version = "8.3"

[composer]
# These record the user's answer to one-time prompts so PHPantom
# does not ask again on every session.

# Generate a minimal composer.json when the project has none.
# generate = true

[stubs]
# Install phpstorm-stubs into the project for projects without Composer.
# install = true

# Override which PHP extension stubs are loaded.
# When unset, PHPantom loads core + all commonly bundled extensions.
# extensions = ["Core", "standard", "json", "mbstring", "curl", "redis"]

[formatting]
# External formatter to proxy. Auto-detected when unset.
# tool = "php-cs-fixer"   # or "phpcbf" or "none"
# timeout = 10000

[diagnostics]
# Enable or disable proxied diagnostic providers.
# Each defaults to true when the corresponding tool is detected
# in the project (e.g. vendor/bin/phpstan exists).

# phpstan = true
# phpmd = false
# php-lint = true
# mago = false
```

## Sections

### `[php]`

| Key       | Type   | Default       | Description                                |
|-----------|--------|---------------|--------------------------------------------|
| `version` | string | auto-detected | PHP version override (e.g. `"8.3"`, `"8.2"`) |

When unset, PHPantom reads the PHP version from `composer.json` (`config.platform.php` or `require.php`). This override exists for projects where `composer.json` is missing or inaccurate.

### `[composer]`

These fields are written by PHPantom when the user responds to a prompt. They can also be set by hand.

| Key        | Type | Default | Description                                             |
|------------|------|---------|---------------------------------------------------------|
| `generate` | bool | unset   | Whether to generate a minimal `composer.json` if missing |

When a key is unset, PHPantom will prompt the user. Once the user answers, PHPantom writes the value so the prompt does not appear again.

### `[stubs]`

| Key          | Type         | Default     | Description                                       |
|--------------|--------------|-------------|---------------------------------------------------|
| `install`    | bool         | unset       | Whether to install phpstorm-stubs for non-Composer projects |
| `extensions` | string array | auto-detect | Which PHP extension stubs to load (see below)     |

Same prompt-and-remember behaviour as the `[composer]` keys for `install`.

#### Extension stub selection

By default PHPantom loads stubs for PHP core and all bundled extensions
(matching the set that ships enabled in a stock PHP build), plus any
extensions declared in the project's `composer.json`. The `extensions`
key lets the user override this entirely.

##### Auto-detection from `composer.json`

When `extensions` is unset, PHPantom reads the `require` and
`require-dev` sections of the project's `composer.json` and collects
every `ext-*` key. These are added on top of the default set.

For example, if `composer.json` contains:

```json
{
    "require": {
        "php": "^8.2",
        "ext-redis": "*",
        "ext-imagick": "*"
    }
}
```

PHPantom loads the default bundled extensions plus `redis` and
`imagick` stubs automatically. No `.phpantom.toml` configuration
needed.

Only `composer.json` is read, not `composer.lock`. Transitive
`ext-*` requirements pulled in by dependencies are intentionally
ignored. Those extensions are used by vendor code, which PHPantom
already skips for diagnostics and does not complete into. If the
user's own code references an extension without declaring it in
`composer.json`, the correct fix is to add the `ext-*` requirement
(or override via `[stubs] extensions` in `.phpantom.toml`).

##### Manual override

```toml
[stubs]
extensions = [
  "Core", "standard", "json", "mbstring", "curl",
  "redis", "imagick", "mongodb",
]
```

When `extensions` is set, only the listed extensions are loaded.
The auto-detection from `composer.json` is skipped entirely. This
is useful when the user wants full control or when the project
has no `composer.json`.

The available extension names match the directory names in
phpstorm-stubs (e.g. `"redis"`, `"imagick"`, `"swoole"`, `"mongodb"`).
An unrecognised name is silently ignored with a log message.

**Implementation note:** The build script already embeds all stub files.
Filtering happens at runtime: when building the stub class/function
indices, skip entries whose source file path does not start with one
of the enabled extension directories. This is a simple string prefix
check on the relative path from `STUB_CLASS_MAP`.

### `[formatting]`

Controls formatting proxy behaviour. PHPantom does not ship a formatter;
it proxies requests to an external tool.

| Key       | Type   | Default     | Description                                        |
|-----------|--------|-------------|----------------------------------------------------|
| `tool`    | string | auto-detect | `"php-cs-fixer"`, `"phpcbf"`, or `"none"`          |
| `timeout` | int    | 10000       | Maximum runtime in milliseconds                    |

"Auto-detect" means PHPantom checks for `vendor/bin/php-cs-fixer` first,
then `vendor/bin/phpcbf`, then the tools on `$PATH`. The first one found
is used. Setting `tool = "none"` disables formatting entirely (PHPantom
does not register the capability).

### `[diagnostics]`

Controls which external tools PHPantom proxies for diagnostics.

| Key        | Type | Default     | Description                          |
|------------|------|-------------|--------------------------------------|
| `phpstan`  | bool | auto-detect | Proxy PHPStan diagnostics            |
| `phpmd`    | bool | auto-detect | Proxy PHP Mess Detector diagnostics  |
| `php-lint` | bool | auto-detect | Proxy `php -l` syntax checking       |
| `mago`     | bool | auto-detect | Proxy Mago diagnostics               |

"Auto-detect" means PHPantom enables the provider when it finds the tool (e.g. `vendor/bin/phpstan` or `phpstan` on `$PATH`). Setting a key to `false` disables it regardless. Setting it to `true` enables it even if auto-detection fails (the user is responsible for making the tool available).

## Design decisions

1. **No global config.** Everything is per-project. Different projects have different tools, different PHP versions, different Composer setups. A global config would create confusing precedence rules.

2. **Prompt-and-remember pattern.** For one-time setup actions (generating `composer.json`, optimizing autoload, installing stubs), PHPantom asks once and records the answer. The user can change their mind by editing the file.

3. **Flat diagnostics for now.** Each diagnostic tool is a simple bool. When we add proxying, individual tools can grow into sub-tables if needed (e.g. `[diagnostics.phpstan]` with `level`, `config`, `memory-limit`). Starting flat avoids premature structure.

4. **No editor or completion knobs.** PHPantom has no user-facing settings for completion behaviour today. Add sections when there is a real need, not speculatively.

## Implementation order

1. **Config writing.** When PHPantom prompts the user and gets an answer, write or update the relevant key. Preserve comments and formatting (use `toml_edit` crate).
2. **Diagnostic proxying.** Wire `[diagnostics]` toggles into the proxy infrastructure as each provider is implemented. *Partially done — `unresolved-member-access` toggle is wired; external tool toggles (`phpstan`, `phpmd`, etc.) await proxy infrastructure.*