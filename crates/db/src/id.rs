//! Ids that leave the app: in URLs and JSON. Internal ids stay `i64`.

use std::{cmp::Ordering, fmt, hash, marker::PhantomData, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sqlx::{Database, Decode, Encode, Type, encode::IsNull, error::BoxDynError};
use uuid::Uuid as RawUuid;

/// A UUIDv7 that names a row of `T` outside the app. `Uuid<Team>` where
/// `Uuid<Org>` is expected does not compile.
///
/// Stored as `uuid` in Postgres, `blob` in SQLite and `binary(16)` in MySQL
/// (sqlx's `uuid::Uuid` encoding). Generate it with [`Uuid::new`] on insert.
/// Serializes, displays and parses as the hyphenated string.
pub struct Uuid<T>(RawUuid, PhantomData<fn() -> T>);

impl<T> Uuid<T> {
    /// A new time-ordered id (`uuid::Uuid::now_v7`).
    #[allow(clippy::new_without_default, reason = "a default id would be a random one; spell it out")]
    pub fn new() -> Self {
        Self::from_uuid(RawUuid::now_v7())
    }

    pub const fn from_uuid(id: RawUuid) -> Self {
        Self(id, PhantomData)
    }

    pub const fn as_uuid(&self) -> RawUuid {
        self.0
    }
}

// Manual impls: derives would require `T` to implement each trait too.
impl<T> Clone for Uuid<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Uuid<T> {}
impl<T> PartialEq for Uuid<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for Uuid<T> {}
impl<T> PartialOrd for Uuid<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Uuid<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}
impl<T> hash::Hash for Uuid<T> {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}
impl<T> fmt::Debug for Uuid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid({})", self.0)
    }
}
impl<T> fmt::Display for Uuid<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl<T> FromStr for Uuid<T> {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RawUuid::try_parse(s).map(Self::from_uuid)
    }
}

impl<T> From<RawUuid> for Uuid<T> {
    fn from(id: RawUuid) -> Self {
        Self::from_uuid(id)
    }
}

impl<T> Serialize for Uuid<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0.hyphenated())
    }
}

/// From a string only: `"42"` or a number is an error, so a numeric id in a
/// path parameter does not parse.
impl<'de, T> Deserialize<'de> for Uuid<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// What `#[sqlx(transparent)]` would write, without bounds on `T`.
impl<T, DB: Database> Type<DB> for Uuid<T>
where
    RawUuid: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <RawUuid as Type<DB>>::type_info()
    }
    fn compatible(ty: &DB::TypeInfo) -> bool {
        <RawUuid as Type<DB>>::compatible(ty)
    }
}

impl<'q, T, DB: Database> Encode<'q, DB> for Uuid<T>
where
    RawUuid: Encode<'q, DB>,
{
    fn encode_by_ref(&self, buf: &mut <DB as Database>::ArgumentBuffer<'q>) -> Result<IsNull, BoxDynError> {
        self.0.encode_by_ref(buf)
    }
}

impl<'r, T, DB: Database> Decode<'r, DB> for Uuid<T>
where
    RawUuid: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        RawUuid::decode(value).map(Self::from_uuid)
    }
}

/// What the frontend sees: `Uuid<"Team">`, a string branded with the Rust
/// type's name, so a `Uuid<"Org">` where a `Uuid<"Team">` is expected fails
/// to type-check. Declared once in `Uuid.ts`; generated types import it.
#[cfg(feature = "ts")]
impl<T: 'static> ts_rs::TS for Uuid<T> {
    type WithoutGenerics = Uuid<()>;
    type OptionInnerType = Self;

    fn docs() -> Option<String> {
        Some("/**\n * A row's id outside the app: a UUIDv7 string, branded with the type it names.\n */\n".into())
    }
    fn ident(_: &ts_rs::Config) -> String {
        "Uuid".into()
    }
    fn name(_: &ts_rs::Config) -> String {
        format!("Uuid<\"{}\">", brand::<T>())
    }
    fn inline(cfg: &ts_rs::Config) -> String {
        Self::name(cfg)
    }
    fn decl(_: &ts_rs::Config) -> String {
        "type Uuid<T extends string> = string & { readonly __uuid: T };".into()
    }
    fn decl_concrete(cfg: &ts_rs::Config) -> String {
        Self::decl(cfg)
    }
    fn output_path() -> Option<std::path::PathBuf> {
        Some("Uuid.ts".into())
    }
}

/// `teams::Invitation<(), ()>` -> `Invitation`.
#[cfg(feature = "ts")]
fn brand<T>() -> &'static str {
    let path = std::any::type_name::<T>();
    let path = path.split('<').next().unwrap_or(path);
    path.rsplit("::").next().unwrap_or(path)
}
