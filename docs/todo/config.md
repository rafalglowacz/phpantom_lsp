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

# Add "optimize-autoload": true to composer.json config.
# optimize-autoload = true

[stubs]
# Install phpstorm-stubs into the project for projects without Composer.
# install = true

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

| Key                  | Type | Default | Description                                             |
|----------------------|------|---------|---------------------------------------------------------|
| `generate`           | bool | unset   | Whether to generate a minimal `composer.json` if missing |
| `optimize-autoload`  | bool | unset   | Whether to add optimize-autoload to `composer.json`      |

When a key is unset, PHPantom will prompt the user. Once the user answers, PHPantom writes the value so the prompt does not appear again.

### `[stubs]`

| Key       | Type | Default | Description                                       |
|-----------|------|---------|---------------------------------------------------|
| `install` | bool | unset   | Whether to install phpstorm-stubs for non-Composer projects |

Same prompt-and-remember behaviour as the `[composer]` keys.

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

1. **Config loading.** Read `.phpantom.toml` from the workspace root on `initialized`. Deserialize with `toml` + `serde`. Missing file means all defaults.
2. **Config writing.** When PHPantom prompts the user and gets an answer, write or update the relevant key. Preserve comments and formatting (use `toml_edit` crate).
3. **PHP version override.** Wire `[php].version` into the existing version detection path.
4. **Diagnostic proxying.** Wire `[diagnostics]` toggles into the proxy infrastructure as each provider is implemented.