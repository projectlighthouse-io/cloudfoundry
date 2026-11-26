# validate

Validate input and prove it with `Valid<T>`.

```toml
[dependencies]
validate = { git = "https://github.com/projectlighthouse-io/cloudfoundry", features = ["route"] }
```

```rust
use validate::{Errors, Valid, Validate, max_len, required};

#[derive(serde::Deserialize)]
struct CreatePipeline { name: String }

impl Validate for CreatePipeline {
    fn validate(&self, e: &mut Errors) {
        e.check("name", required(&self.name)).check("name", max_len(&self.name, 80));
    }
}

// feature `route`: a 422 with every error, before the handler runs
async fn store(input: Valid<route::Json<CreatePipeline>>) -> String { input.name.clone() }
```

Rules are plain functions returning `Result<(), Message>`: `required`, `min_len`,
`max_len`, `between`, `one_of`, `email`.

## Features

| Feature | Adds |
| --- | --- |
| `route` | `Valid<Form|Json|Query|Path<T>>` as handler inputs; `Errors` as a 422 response |
