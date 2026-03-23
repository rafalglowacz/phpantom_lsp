# PHPantom — Bug Fixes

Known bugs and incorrect behaviour. These are distinct from feature
requests — they represent cases where existing functionality produces
wrong results. Bugs should generally be fixed before new features at
the same impact tier.

Items are ordered by **impact** (descending), then **effort** (ascending)
within the same impact tier.

| Label      | Scale                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------- |
| **Impact** | **Critical**, **High**, **Medium-High**, **Medium**, **Low-Medium**, **Low**                                           |
| **Effort** | **Low** (≤ 1 day), **Medium** (2-5 days), **Medium-High** (1-2 weeks), **High** (2-4 weeks), **Very High** (> 1 month) |

---

#### B10. Negative narrowing after early return not applied

| | |
|---|---|
| **Impact** | Low-Medium |
| **Effort** | Medium |

After `if ($x instanceof Y) { return; }`, the variable `$x` should be
narrowed to exclude `Y` for all subsequent code in the same scope.
PHPantom does not apply this negative narrowing via early return.

**Reproduce:**

```php
public static function toString(mixed $value): string
{
    if ($value instanceof Stringable) {
        return $value->__toString();
    }
    if ($value instanceof BackedEnum) {
        $value = $value->value; // PHPantom resolves $value as Stringable here
    }
}
```

The diagnostic reports `Property 'value' not found on class 'Stringable'`
because `$value` is still resolved as `Stringable` inside the
`BackedEnum` branch, even though that branch is only reachable when
`$value` is NOT `Stringable`.

Guard-clause narrowing already works for `if (!$x instanceof Y) { return; }`
(positive narrowing after negated check). This is the inverse: positive
check with exit should produce negative narrowing for subsequent code.

**Triage count:** ~2 diagnostics directly, but a general correctness
issue that affects any code using the early-return-after-instanceof
pattern.

