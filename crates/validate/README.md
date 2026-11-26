# validate

Validate input and prove it with `Valid<T>`: a value that exists only if every
rule passed. All rules run; errors are collected per field.

```toml
[dependencies]
validate = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["route"] }
```

```rust
use validate::{Errors, Valid, Validate, email, max_len, required};

#[derive(serde::Deserialize)]
struct CreateUser { name: String, email: String }

impl Validate for CreateUser {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name)).check("name", max_len(&self.name, 80));
        e.check("email", email(&self.email));
    }
}

// feature `route`: invalid input is a 422 with every error, before the handler runs
async fn store(input: Valid<route::Json<CreateUser>>) -> String { input.name.clone() }
```

A failed request gets:

```json
{ "message": "The given data was invalid.", "errors": { "email": ["Must be a valid email address."] } }
```

With `inertia` (feature `validate`), Inertia visits are redirected back and the
next page gets `props.errors` instead.

## Rules

| Kind | Rules |
| --- | --- |
| Presence, length | `required`, `min_len`, `max_len`, `exact_len`, `min_items`, `max_items` |
| Numbers | `between`, `at_least`, `at_most`, `integer`, `numeric` |
| Choices, comparisons | `one_of`, `not_one_of`, `same`, `different`, `accepted` |
| Characters | `alpha`, `alpha_num`, `alpha_dash`, `lowercase`, `uppercase`, `starts_with`, `ends_with` |
| Formats | `email`, `url`, `uuid`, `ip`, `ipv4`, `ipv6`, `date` (`YYYY-MM-DD`) |

## Custom rules

A rule is any function returning `Result<(), Message>`:

```rust
fn repo_slug(s: &str) -> Result<(), Message> {
    rule(s.split('/').count() == 2, "Must look like owner/name.")
}

e.check("repo", repo_slug(&self.repo));                                  // own rule
e.check("name", required(&self.name).or_message("Tell us your name.")); // own message
e.check("confirm", same(&self.password, &self.confirm));                 // cross-field
e.check("bio", nullable(self.bio.as_deref(), |b| max_len(b, 500)));      // optional field
e.check(&format!("tags.{i}"), required(tag));                            // per item
```

Database checks ("email is taken") belong in an action; report them with
`Errors::add`. Full guide: the crate docs (`cargo doc -p validate --open`).

## Features

| Feature | Adds |
| --- | --- |
| `route` | `Valid<Form\|Json\|Query\|Path<T>>` as handler inputs; `Errors` as a 422 response |
