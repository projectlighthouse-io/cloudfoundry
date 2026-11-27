# validate

Validate input and prove it with `Valid<T>`: a value that exists only if every
rule passed. Rules are attributes on the struct; errors are collected per field,
keyed the way the client named them.

```toml
[dependencies]
validate = { git = "https://github.com/projectlighthouse-io/toolkit", features = ["route"] }
```

```rust
use serde::Deserialize;
use validate::{Valid, Validate};

#[derive(Deserialize, Validate)]
struct CreateUser {
    #[validate(required, max = 80)]
    name: String,
    #[validate(required, email)]
    email: String,
    #[validate(nullable, url)]
    website: Option<String>,
}

// feature `route`: invalid input is a 422 with every error, before the handler runs
async fn store(input: Valid<route::Json<CreateUser>>) -> String { input.name.clone() }
```

A failed request gets:

```json
{ "message": "The given data was invalid.", "errors": { "email": ["Must be a valid email address."] } }
```

With `inertia` (feature `validate`), Inertia visits are redirected back and the
next page gets `props.errors` instead. Each field reports its first failure.

## Rules

| Attribute | Checks |
| --- | --- |
| `required` | Text not blank, list not empty, `Option` is `Some` |
| `nullable` | Documents an `Option` field: `None` passes |
| `min = n`, `max = n`, `between(a, b)` | Characters for text, items for lists, the value for numbers |
| `email`, `url`, `uuid`, `date` | Text shapes |
| `integer`, `numeric` | Numbers written as text |
| `one_of("a", "b")` | One of a fixed set |
| `same = other_field` | Equal to another field (password confirmation) |
| `accepted` | `true` |
| `rule = MyRule` | Your own `Rule` |
| `with = function` | `fn(&Self, &Field) -> Result<(), Message>`, for rules that read other fields |

Error keys follow `#[serde(rename)]` and `#[serde(rename_all)]`.

## Your own rules

```rust
struct RepoSlug;

impl Rule<str> for RepoSlug {
    fn name(&self) -> &str { "repo_slug" }
    fn check(&self, value: &str) -> Result<(), Message> {
        rule(value.split_once('/').is_some(), "Use owner/name.")
    }
}

#[derive(Deserialize, Validate)]
#[validate(with = cross_checks, messages = messages)]
struct NewPipeline {
    #[validate(required, rule = RepoSlug)]
    repo: String,
}

fn cross_checks(p: &NewPipeline, e: &mut Errors) { /* e.field("x", &p.x).rule(min(1)) */ }
fn messages() -> Messages { Messages::new().set("repo.repo_slug", "Which repository?") }
```

`#[validate(with = f)]` on the struct runs after the field rules and can report
on any field. `messages = f` rewords rules by `field.rule`. Checks that need the
database ("email is taken") or the logged-in user belong in the handler, after
`Valid<T>`; report them with `Errors::add`. Full guide: `cargo doc -p validate --open`.

## Features

| Feature | Adds |
| --- | --- |
| `derive` (default) | `#[derive(Validate)]` |
| `route` | `Valid<Form\|Json\|Query\|Path<T>>` as handler inputs; `Errors` as a 422 response |
