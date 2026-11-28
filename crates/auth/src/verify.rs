//! Email verification: a link with a random token (only its SHA-256 hash is
//! stored), for a new account's address or for a new address the user wants
//! to change to. [`Verified<U>`](crate::Verified) guards routes.

use std::{fmt, future::Future, time::Duration};

use sha2::Digest;

use crate::{Auth, Credentials, Emitted, Error, events, random_token, unix_now};

/// How long a verification link works.
pub const VERIFY_EXPIRES: Duration = Duration::from_secs(60 * 60);
/// How often a user can be sent a new link.
pub const VERIFY_THROTTLE: Duration = Duration::from_secs(60);

/// A link to store: who it is for, the address it verifies, its token's hash.
#[derive(Clone, Debug)]
pub struct NewVerification<Id> {
    pub user: Id,
    pub email: String,
    /// SHA-256 of the token in the link.
    pub token_hash: Vec<u8>,
    /// Unix seconds.
    pub expires_at: i64,
    pub created_at: i64,
}

/// A stored link, found by its token's hash.
#[derive(Clone, Debug)]
pub struct Verification<Id> {
    pub id: i64,
    pub user: Id,
    pub email: String,
    pub expires_at: i64,
    pub verified_at: Option<i64>,
}

/// Providers that keep email verification links, for [`verification_token`]
/// and [`Auth::verify_email`].
pub trait Verifications: Credentials {
    /// The user's current address.
    fn email(&self, user: &Self::User) -> String;
    /// Unix seconds; `None` until the current address is verified.
    fn email_verified_at(&self, user: &Self::User) -> Option<i64>;
    /// Keep a new link. Older unused links of the user may be deleted.
    fn store_verification(&self, link: NewVerification<Self::Id>) -> impl Future<Output = Result<(), Error>> + Send;
    /// When the user's newest link was made (unix seconds), for the resend limit.
    fn last_verification_at(&self, id: Self::Id) -> impl Future<Output = Result<Option<i64>, Error>> + Send;
    fn verification_by_token(&self, token_hash: &[u8]) -> impl Future<Output = Result<Option<Verification<Self::Id>>, Error>> + Send;
    /// In one transaction: mark the link used (only if it is unused), set the
    /// user's email to `email` and `email_verified_at` to `now`. `false` if
    /// the link was already used.
    fn complete_verification(&self, link: i64, user: Self::Id, email: &str, now: i64) -> impl Future<Output = Result<bool, Error>> + Send;
}

#[derive(Debug)]
pub enum VerifyError {
    /// The current address is already verified (and no new one was given).
    AlreadyVerified,
    /// A link was made less than [`VERIFY_THROTTLE`] ago; try again after this long.
    Throttled(Duration),
    /// Another account has this address.
    EmailTaken,
    /// The token is wrong, used, or older than [`VERIFY_EXPIRES`].
    InvalidToken,
    Other(Error),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyVerified => f.write_str("the email address is already verified"),
            Self::Throttled(d) => write!(f, "a verification link was sent moments ago; try again in {} seconds", d.as_secs().max(1)),
            Self::EmailTaken => f.write_str("the email address is taken"),
            Self::InvalidToken => f.write_str("this verification link is invalid or has expired"),
            Self::Other(e) => write!(f, "email verification failed: {e}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// SHA-256 of a verification token, as stored.
pub fn hash_verification_token(token: &str) -> Vec<u8> {
    sha2::Sha256::digest(token.as_bytes()).to_vec()
}

/// Make a verification link token for `user`: for their current address, or
/// with `new_email` for an address to change to (it replaces the current one
/// only once verified). Returns the plain token for the link; send the mail to
/// the address being verified. At most one link per [`VERIFY_THROTTLE`].
pub async fn verification_token<P: Verifications>(users: &P, user: &P::User, new_email: Option<&str>) -> Result<String, VerifyError> {
    let email = match new_email.map(str::trim) {
        Some(new) => {
            if let Some(other) = users.by_login(new).await.map_err(VerifyError::Other)? {
                if !same_user(users, &other, user) {
                    return Err(VerifyError::EmailTaken);
                }
            }
            new.to_owned()
        }
        None if users.email_verified_at(user).is_some() => return Err(VerifyError::AlreadyVerified),
        None => users.email(user),
    };
    let now = unix_now();
    if let Some(last) = users.last_verification_at(users.id(user)).await.map_err(VerifyError::Other)? {
        let wait = VERIFY_THROTTLE.as_secs() as i64 - (now - last);
        if wait > 0 {
            return Err(VerifyError::Throttled(Duration::from_secs(wait as u64)));
        }
    }
    let token = random_token();
    let link = NewVerification {
        user: users.id(user),
        email,
        token_hash: hash_verification_token(&token),
        expires_at: now + VERIFY_EXPIRES.as_secs() as i64,
        created_at: now,
    };
    users.store_verification(link).await.map_err(VerifyError::Other)?;
    Ok(token)
}

fn same_user<P: Verifications>(users: &P, a: &P::User, b: &P::User) -> bool {
    serde_json::to_value(users.id(a)).ok() == serde_json::to_value(users.id(b)).ok()
}

impl<U: Clone + Send + Sync + 'static> Auth<U> {
    /// Follow a verification link: the address is verified (and becomes the
    /// user's email, for a change of address). Works without being logged in:
    /// the token is the proof. Emits `EmailVerified`. Returns the user.
    pub async fn verify_email<P>(&self, users: &P, token: &str) -> Result<U, VerifyError>
    where
        P: Verifications<User = U>,
    {
        let link = users.verification_by_token(&hash_verification_token(token)).await.map_err(VerifyError::Other)?;
        let Some(link) = link.filter(|l| l.verified_at.is_none() && l.expires_at > unix_now()) else { return Err(VerifyError::InvalidToken) };
        let user = users.find(link.user).await.map_err(VerifyError::Other)?.ok_or(VerifyError::InvalidToken)?;
        if let Some(other) = users.by_login(&link.email).await.map_err(VerifyError::Other)? {
            if !same_user(users, &other, &user) {
                return Err(VerifyError::EmailTaken);
            }
        }
        let changed = users.email(&user) != link.email;
        if !users.complete_verification(link.id, users.id(&user), &link.email, unix_now()).await.map_err(VerifyError::Other)? {
            return Err(VerifyError::InvalidToken);
        }
        let user = users.find(users.id(&user)).await.map_err(VerifyError::Other)?.ok_or(VerifyError::InvalidToken)?;
        if let Some(user_id) = serde_json::to_value(users.id(&user)).ok().and_then(|v| v.as_i64()) {
            self.cx.emit(Emitted::EmailVerified(events::EmailVerified { user_id, email: link.email, changed })).await;
        }
        Ok(user)
    }
}
