# PHPantom — Bug Fixes

## B6: Scope methods not found on Builder in analyzer chains

PHPantom's completion engine correctly injects scope methods onto
`Builder<ConcreteModel>` via `try_inject_builder_scopes` in
`resolve_named_type`. However, the analyzer's `check_member_on_resolved_classes`
uses `resolve_class_fully_cached` which is keyed by bare FQN without
generic args. A prior cache entry for `Builder` (without model-specific
scopes) is returned, and the scope method is reported as not found.

The analyzer does check `base_classes` first (before the cache) to avoid
this, but in method chains like
`ArticleCategoryTranslation::whereHas(...)->whereLanguage(...)`, the
intermediate `Builder<ArticleCategoryTranslation>` type produced by
`whereHas()` may not carry the scope-injected methods in `base_classes`.

Affected diagnostics (5 direct + 2 cascading):

Direct `unknown_member` — scope method exists on model but not found on
Builder:
- `ArticleRepository:69` — `whereLanguage` (scope on
  `ArticleCategoryTranslation`)
- `ProductRepository:271` — `whereIsLuxury` (scope on `Product`)
- `ProductRepository:272` — `whereIsDerma` (scope on `Product`)
- `ProductRepository:273` — `whereIsProHairCare` (scope on `Product`)
- `ProductRepository:369` — `whereIsLuxury` (scope on `Product`)

Cascading `unresolved_member_access`:
- `EventRepository:23` — `pluck` after broken
  `whereIsBlackFriday()->whereIsVisible()` chain

Note: `EventRepository:22` reports `whereIsVisible` not found on Builder.
Product has `scopeIsVisibleIn` (takes a `Country` parameter) but no
`scopeWhereIsVisible` and no `is_visible` column. This may be a genuine
code bug in the project rather than an LSP issue.

**Impact:** 5–6 direct `unknown_member` diagnostics plus 1–2 cascading.

## B7: PHPDoc `@param` generic array type not merged with native `array` hint

When a method has a native type hint `array` and a PHPDoc `@param` with
a generic type like `list<Request>`, PHPantom doesn't merge the PHPDoc
element type with the native `array` for narrowing purposes. After an
`is_array()` guard, the variable narrows to `array` but loses the `Request`
element type from the docblock.

Real-world example — `MobilePayConnection.php`:

```php
/**
 * @param null|list<Request>|Request $request
 */
protected function connect(string $uri, null|array|Request $request, ...): MobilePayResponse
{
    if (is_array($request)) {
        foreach ($request as $item) {
            $serializedObjects[] = $item->jsonSerialize();
            //                     ^^^^^ unresolved
        }
    }
}
```

After `is_array($request)`, `$request` narrows from `null|array|Request`
to `array`. The `@param` says the array case is `list<Request>`, so
`$item` should be `Request`. But the LSP doesn't unify the narrowed
native `array` with the docblock's `list<Request>`.

**Impact:** 1 diagnostic in the shared project
(`MobilePayConnection:76`).

## B9: Eloquent relationship property lookup is case-sensitive

Laravel normalises property names via `Str::snake()` at runtime, so
`$order->orderProducts` and `$order->orderproducts` both resolve to the
same relationship. PHPantom's property lookup is case-sensitive, so when
code uses `orderProducts` (camelCase) but the model declares the
relationship method and `@property` as `orderproducts` (all lowercase),
the property is not found.

Real-world example — `FlowService.php`:

```php
// FlowService line 477:
$items = $order->orderProducts->map(...);
//              ^^^^^^^^^^^^^^ camelCase — not found

// Order model declares:
public function orderproducts(): HasMany { ... }
// and @property uses 'orderproducts' (lowercase)
```

The fix should apply `Str::snake()`-equivalent normalisation (or
case-insensitive matching) when looking up relationship-derived virtual
properties on Eloquent models.

**Impact:** 1 direct diagnostic (`FlowService:477`) plus 1 cascading
(`FlowService:517` — compound with `Collection::reduce()` type loss).



