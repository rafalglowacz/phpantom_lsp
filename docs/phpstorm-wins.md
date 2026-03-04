# PHPantom vs PHPStorm — Where PHPantom Wins

Cases from `example.php` where PHPantom resolves correctly but PHPStorm
(2024.3) does not. Each entry references the original line number in
`example.php` and describes what PHPStorm fails to do.

Tested: full file.

---

## `@var` without variable name (L80–81)

```php
/** @var User */
$hinted = getUnknownValue();
$hinted->getEmail();    // PHPStorm: unresolved. PHPantom: User
```

PHPStorm requires the variable name in the `@var` tag
(`/** @var User $hinted */`). PHPantom resolves either form.

---

## Conditional return types — `class-string<T> → T` (L209–213)

```php
$resolved = $container->make(User::class);
$resolved->getEmail();    // PHPStorm: unresolved. PHPantom: User

$appUser = app(User::class);
$appUser->getEmail();     // PHPStorm: unresolved. PHPantom: User
```

PHPStorm does not evaluate `($abstract is class-string<TClass> ? TClass : mixed)`
conditional return types from PHPStan-style docblocks. PHPantom resolves
`TClass` to the concrete class passed at the call site.

---

## Negated `instanceof` narrowing (L258–266)

```php
$a = findOrFail(1);    // User|AdminUser
if ($a instanceof AdminUser) {
    $a->grantPermission('x');    // both resolve
} else {
    $a->getEmail();              // PHPStorm: unresolved. PHPantom: User
}

if (!$a instanceof AdminUser) {
    $a->getEmail();              // PHPStorm: unresolved. PHPantom: User
}
```

PHPStorm does not narrow the else branch of `instanceof` or the body of
`!$a instanceof`. PHPantom subtracts the checked type from the union.

---

## `@phpstan-assert` type narrowing (L297–311)

```php
assertUser($i);                  // @phpstan-assert User $value
$i->getEmail();                  // PHPStorm: unresolved. PHPantom: User

if (isAdmin($j)) {               // @phpstan-assert-if-true AdminUser
    $j->grantPermission('sudo');
} else {
    $j->getEmail();              // PHPStorm: unresolved. PHPantom: User
}

if (isRegularUser($k)) {         // @phpstan-assert-if-false AdminUser
    $k->getEmail();              // PHPStorm: unresolved. PHPantom: User
} else {
    $k->grantPermission('x');
}
```

PHPStorm does not read `@phpstan-assert`, `@phpstan-assert-if-true`, or
`@phpstan-assert-if-false` tags. PHPantom narrows (or subtracts) the
asserted type in the appropriate branch.

---

## Trait generic substitution — `HasFactory` (L380–382)

```php
Product::factory()->create();             // PHPStorm: unresolved. PHPantom: Product
Product::factory()->count(5)->make();     // PHPStorm: unresolved. PHPantom: Product
Product::factory()->state([])->create();  // PHPStorm: unresolved. PHPantom: Product
```

PHPStorm does not resolve generics through `@use HasFactory<UserFactory>`
and the factory's `@extends Factory<Product>`. PHPantom substitutes
`TModel` through the full chain: trait use annotation, factory class
generics, and `static` return types.

---

## Trait generic substitution — `Indexable` (L384–385)

```php
$idx = new UserIndex();       // @use Indexable<int, User>
$idx->get()->getEmail();      // PHPStorm: unresolved. PHPantom: User
```

Same mechanism as above. PHPStorm does not substitute `TValue` from
`@use Indexable<int, User>` into the trait method's return type.

---

## Array element type from `@var` without variable name (L406–414)

```php
/** @var array<int, User> */
$annotated = [];
$annotated[0]->getEmail();        // PHPStorm: unresolved. PHPantom: User

/** @var list<AdminUser> */
$listed = [];
$listed[0]->grantPermission('z'); // PHPStorm: unresolved. PHPantom: AdminUser

$inferred = [new User('', ''), new AdminUser('', '')];
$inferred[0]->getName();          // PHPStorm: unresolved. PHPantom: User|AdminUser
```

PHPStorm fails to extract element types from `@var` annotations that
don't include the variable name, and does not infer element types from
array literal contents.

---

## Array destructuring element type (L424–425)

```php
/** @var list<User> */
[$first, $second] = getUnknownValue();
$first->getEmail();    // PHPStorm: unresolved. PHPantom: User
```

PHPStorm does not propagate the element type of a `list<T>` through
array destructuring syntax.

---

## Array shape value type resolution (L431–432)

```php
$config = ['host' => 'localhost', 'port' => 3306, 'author' => new User('', '')];
$config['author']->getEmail();    // PHPStorm: unresolved. PHPantom: User
```

PHPStorm does not infer the type of individual keys in an array literal
used as a shape.

---

## `$_SERVER` superglobal key completion (L467–469)

```php
$_SERVER['REQUEST_METHOD'];    // PHPStorm: no key completion
$_SERVER['HTTP_HOST'];
$_SERVER['REMOTE_ADDR'];
```

PHPStorm does not offer key completion for `$_SERVER`. PHPantom provides
known key names from its built-in shape definition.

---

## Callable snippet insertion (L560–564)

```php
$user->setName('Bob');            // PHPantom inserts: setName(${1:$name})
$user->addRoles();                // PHPantom inserts: addRoles() (variadic → no tab-stops)
User::findByEmail('a@b.c');       // PHPantom inserts: findByEmail(${1:$email})
$r = new Response(200);           // PHPantom inserts: Response(${1:$statusCode})
```

PHPStorm inserts method names without parameter tab-stops. PHPantom
generates LSP snippet syntax with numbered tab-stops for each required
parameter, skipping optional and variadic parameters.

---

## Spread operator element type tracking (L650–675)

```php
/** @var list<User> $users */
$allUsers = [...$users];
$allUsers[0]->getEmail();                 // PHPStorm: unresolved. PHPantom: User

$everyone = [...$users, ...$admins];
$everyone[0]->getEmail();                 // PHPStorm: unresolved. PHPantom: User|AdminUser

/** @var array<int, User> $indexed */
$copy = [...$indexed];
$copy[0]->getName();                      // PHPStorm: unresolved. PHPantom: User

/** @var User[] $typed */
$merged = [...$typed];
$merged[0]->getEmail();                   // PHPStorm: unresolved. PHPantom: User

$legacy = array(...$users, ...$admins);
$legacy[0]->getEmail();                   // PHPStorm: unresolved. PHPantom: User|AdminUser
```

PHPStorm does not propagate element types through the spread operator in
array literals. PHPantom resolves the element type from the spread
variable's iterable annotation and merges types when multiple spreads are
combined.

---

## Type alias shape key completion (L701–708)

```php
$aliasDemo = new TypeAliasDemo();
$userData = $aliasDemo->getUserData();
$userData['name'];                        // PHPStorm: unresolved. PHPantom: key completion

$importDemo = new TypeAliasImportDemo();
$imported = $importDemo->fetchUser();
$imported['email'];                       // PHPStorm: unresolved. PHPantom: key completion
```

PHPStorm does not resolve `@phpstan-type` or `@phpstan-import-type`
aliases into array shape keys for completion. PHPantom follows the alias
chain and offers the shape's keys.

---

## Eloquent virtual members — full suite (L872–948)

PHPStorm does not resolve any of the following Eloquent virtual member
categories. PHPantom resolves all of them.

**Column name properties** from `$fillable`, `$guarded`, `$hidden`:
```php
$author->name;                    // PHPStorm: unresolved. PHPantom: mixed
$author->email;                   // PHPStorm: unresolved. PHPantom: mixed
```

**`$casts` type resolution** (built-in casts, enum casts, custom cast classes):
```php
$author->is_admin;                // PHPStorm: unresolved. PHPantom: bool
$author->created_at;              // PHPStorm: unresolved. PHPantom: Carbon
$author->status;                  // PHPStorm: unresolved. PHPantom: OrderStatus (enum)
$author->description->toHtml();   // PHPStorm: unresolved. PHPantom: HtmlString (custom cast)
```

**`casts()` method** (overrides/extends `$casts` property):
```php
$author->verified_at;             // PHPStorm: unresolved. PHPantom: Carbon
```

**`$attributes` default types** (inferred from literal values):
```php
$author->role;                    // PHPStorm: unresolved. PHPantom: string
$author->is_active;               // PHPStorm: unresolved. PHPantom: bool
$author->login_count;             // PHPStorm: unresolved. PHPantom: int
```

**Relationship virtual properties**:
```php
$author->posts;                   // PHPStorm: unresolved. PHPantom: Collection<BlogPost>
$author->profile;                 // PHPStorm: unresolved. PHPantom: AuthorProfile
$author->profile->getBio();       // PHPStorm: unresolved. PHPantom: chains through
$author->commentable;             // PHPStorm: unresolved. PHPantom: Model (body-inferred morphTo)
```

**Scope methods** (instance and static access, `$query` parameter stripped):
```php
$author->active();                // PHPStorm: unresolved. PHPantom: virtual method
BlogAuthor::ofGenre('fiction');   // PHPStorm: unresolved. PHPantom: static scope
```

**Legacy and modern accessors**:
```php
$author->display_name;            // PHPStorm: unresolved. PHPantom: string (getDisplayNameAttribute)
$author->avatar_url;              // PHPStorm: unresolved. PHPantom: mixed (Attribute accessor)
```

**Builder-as-static forwarding** and query chain resolution:
```php
BlogAuthor::where('active', true);                    // PHPStorm: unresolved. PHPantom: Builder<BlogAuthor>
BlogAuthor::where('active', 1)->get();                // PHPStorm: unresolved. PHPantom: Collection<BlogAuthor>
BlogAuthor::where('active', 1)->first();              // PHPStorm: unresolved. PHPantom: BlogAuthor|null
BlogAuthor::where('active', 1)->first()->profile->getBio(); // PHPStorm: unresolved. PHPantom: full chain
BlogAuthor::whereIn('id', [1, 2])->groupBy('genre')->get(); // PHPStorm: unresolved. PHPantom: @mixin forwarding
```

**Scopes on Builder instances** (scopes remain available after builder chaining):
```php
BlogAuthor::where('active', 1)->active();                     // PHPStorm: unresolved. PHPantom: scope on Builder
BlogAuthor::where('active', 1)->active()->ofGenre('sci-fi')->get(); // PHPStorm: unresolved. PHPantom: multi-scope chain
$q = BlogAuthor::where('genre', 'fiction');
$q->active();                                                 // PHPStorm: unresolved. PHPantom: scope on variable
```

---

## Custom Eloquent collections via `#[CollectedBy]` (L957–968)

```php
$reviews = Review::where('published', true)->get();
$reviews->topRated();             // PHPStorm: unresolved. PHPantom: ReviewCollection method
$reviews->averageRating();        // PHPStorm: unresolved. PHPantom: ReviewCollection method
$reviews->first();                // PHPStorm: unresolved. PHPantom: Review|null

$review->replies->topRated();     // PHPStorm: unresolved. PHPantom: relationship → ReviewCollection
```

PHPStorm does not resolve `#[CollectedBy(ReviewCollection::class)]` to
return the custom collection from `->get()` or relationship properties.
PHPantom resolves the attribute and returns custom collection methods
alongside inherited ones.

---

## Match class-string forwarding to conditional return types (L976–1028)

```php
$requestType = match ($typeName) {
    'reviews' => ElasticProductReviewIndexService::class,
    'brands'  => ElasticBrandIndexService::class,
};
$requestBody = $container->make($requestType);
$requestBody->index();            // PHPStorm: unresolved. PHPantom: shared method
$requestBody->reindex();          // PHPStorm: unresolved. PHPantom: ElasticProductReviewIndexService only

$cls = User::class;
$user = $container->make($cls);
$user->getEmail();                // PHPStorm: unresolved. PHPantom: User

$cls = $flag ? User::class : AdminUser::class;
$obj = $container->make($cls);
$obj->getName();                  // PHPStorm: unresolved. PHPantom: User|AdminUser
```

PHPStorm does not trace class-string values from match expressions,
simple variables, or ternary expressions back through `@template T` +
`@param class-string<T>` + `@return T` signatures. PHPantom resolves
all of these patterns, including inline chains and function calls.

---

## Iterator key types in foreach (L1086–1089)

```php
/** @return array<Request, HttpResponse> */
foreach ($this->getMapping() as $req => $res) {
    $req->getUri();               // PHPStorm: unresolved. PHPantom: Request (key type)
}
```

PHPStorm does not resolve the key variable type from `array<K, V>` in a
foreach loop. PHPantom extracts the key type and resolves it.

---

## Generator `@var` annotation in foreach (L1152–1168)

```php
/** @var \Generator<int, User> $gen */
$gen = $this->getUsers();
foreach ($gen as $user) {
    $user->getEmail();            // PHPStorm: unresolved. PHPantom: User
}

/** @var \Generator<int, User, mixed, Response> $gen */
foreach ($gen as $item) {
    $item->getEmail();            // PHPStorm: unresolved. PHPantom: User (2nd param, not 4th)
}
```

PHPStorm does not resolve the value type from a `@var` annotated
Generator variable used in a foreach. PHPantom extracts the 2nd type
parameter as the iteration value type.

---

## Generator `@param` annotation in foreach (L1188–1194)

```php
/**
 * @param \Generator<int, Customer> $customers
 */
public function foreachGeneratorParam(\Generator $customers): void
{
    foreach ($customers as $customer) {
        $customer->getName();         // PHPStorm: unresolved. PHPantom: Customer
    }
}
```

PHPStorm does not resolve the value type from a `@param` annotated
Generator parameter. PHPantom extracts the element type from the
parameter's generic annotation.

---

## Generator yield type inference inside bodies (L1200–1270)

```php
/** @return \Generator<int, User> */
public function findAll(): \Generator
{
    yield $user;
    $user->getEmail();                // PHPStorm: unresolved. PHPantom: User

    yield 0 => $anotherUser;
    $anotherUser->getName();          // PHPStorm: unresolved. PHPantom: User
}
```

When a function's return type is `Generator<TKey, TValue>`, PHPantom
infers that variables appearing in `yield $var` statements have type
`TValue`. This works inside control flow blocks, with multiple
independent yield variables, and through method chains on the inferred
type. PHPStorm does not perform this inference.

**TSend inference** is also resolved:
```php
/** @return \Generator<int, string, Request, void> */
$request = yield 'ready';
$request->getUri();               // PHPStorm: unresolved. PHPantom: Request (TSend = 3rd param)
```

---

## Nested literal array shape keys (L1317–1322)

```php
$config = ['db' => ['host' => 'localhost', 'port' => 3306], 'debug' => true];
$config['db']['host'];            // PHPStorm: unresolved. PHPantom: nested key completion
$config['debug'];                 // PHPStorm: unresolved. PHPantom: first-level key
```

PHPStorm does not infer nested shape keys from array literal values
without a `@var` annotation. PHPantom analyses the literal structure and
offers keys at each nesting level.

---

## Object shape nested member resolution (L1325–1332)

```php
/** @return object{user: User, meta: object{page: int, total: int}} */
$result = $this->getResult();
$result->user->getEmail();        // PHPStorm: unresolved. PHPantom: User (nested object → class)
```

PHPStorm does not resolve class members when a typed property inside an
object shape refers to a real class. PHPantom resolves `object{user: User}`
and chains into `User` methods.

---

## Shape destructuring with `@var` annotation (L1591–1598)

```php
/** @var array{user: User, profile: UserProfile, active: bool} $data */
['user' => $person, 'profile' => $prof] = $data;
$person->getEmail();              // PHPStorm: unresolved. PHPantom: User
```

PHPStorm does not propagate individual key types from a `@var` annotated
array shape through named destructuring. PHPantom maps each destructured
variable to the corresponding shape key's type.

---

## Array access on method return type (L1826–1831)

```php
$gifts = (new GiftShop())->getGifts();    // returns Gift[]
$gifts[0]->open();                // PHPStorm: unresolved. PHPantom: Gift
```

PHPStorm does not resolve the element type when indexing into an array
returned by a multi-line chained method call. PHPantom extracts the
element type from the `Gift[]` return annotation.

---

## Closure parameter type inference (L1844–1886)

```php
// Arrow function in map():
$this->users->map(fn($u) => $u->getEmail());
// PHPStorm: $u unresolved. PHPantom: User (from callable(TValue, TKey))

// Closure in each():
$this->users->each(function ($user) {
    $user->getEmail();            // PHPStorm: unresolved. PHPantom: User
});

// Eloquent chunk():
BlogAuthor::where('active', true)->chunk(100, function ($orders) {
    $orders->count();             // PHPStorm: unresolved. PHPantom: Eloquent Collection
});

// Eloquent whereHas():
BlogAuthor::whereHas('posts', function ($q) {
    $q->where('published', true); // PHPStorm: unresolved. PHPantom: Builder
});
```

PHPStorm does not infer closure/arrow function parameter types from the
generic signature of the calling method. PHPantom substitutes `TValue`,
`TKey`, and `TModel` from the collection or builder's generic context
into the callable parameter types.

---

## Unset variable suppression (L1738–1773)

```php
$user = new User('Alice', 'alice@example.com');
unset($user);
$user->   // PHPStorm: still offers User completions. PHPantom: no completions

$profile = new UserProfile($user);
unset($user, $profile);
$user->   // PHPStorm: still offers completions. PHPantom: no completions
$profile->// PHPStorm: still offers completions. PHPantom: no completions
```

After `unset($var)`, PHPStorm continues to offer completions for the
variable as if it still held its previous type. PHPantom correctly
removes the variable from scope, producing no completions. This also
works with `unset($a, $b)` targeting multiple variables at once, and
re-assigning the variable after unset restores completions with the new
type.

---

## Score

**PHPantom wins: 26 categories.** PHPStorm (2024.3) fails to resolve in
all of them.

These are areas where PHPantom's PHPStan-aware type engine goes beyond
what PHPStorm provides out of the box: conditional return types, assert
tags, generic substitution through traits and inheritance, Eloquent
virtual members (column properties, casts, relationships, scopes,
accessors, builder forwarding, custom collections), match/ternary
class-string tracing, spread operator tracking, generator yield/send
inference, closure parameter inference, nested shape resolution, unset
suppression, and snippet insertion.