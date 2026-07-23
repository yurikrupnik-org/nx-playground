use crate::error::{UserError, UserResult};
use crate::models::{Role, User};
use crate::oauth::types::{OAuthUserInfo, Provider};
use crate::oauth::{CreateOAuthAccountParams, OAuthAccountRepository};
use crate::password;
use crate::repository::UserRepository;
use chrono::Utc;
use uuid::Uuid;

/// Result of OAuth account linking attempt
#[derive(Debug, Clone)]
pub enum AccountLinkingResult {
    /// New user was created
    NewUser(User),
    /// Existing user was found and logged in
    ExistingUser(User),
    /// Manual linking required (email exists but not verified on both sides)
    LinkRequired {
        existing_user_id: Uuid,
        provider_data: OAuthUserInfo,
    },
}

/// Parameters for an OAuth login callback.
#[derive(Debug, Clone)]
pub struct OAuthLogin {
    pub provider: Provider,
    pub user_info: OAuthUserInfo,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    /// Token lifetime in seconds, as reported by the provider.
    pub expires_in: Option<u64>,
    /// Auto-link to an existing account when BOTH the provider-reported email
    /// and the local account email are verified.
    pub auto_link_verified_emails: bool,
}

/// Service for handling OAuth account linking logic
#[derive(Clone)]
pub struct AccountLinkingService<R: UserRepository, O: OAuthAccountRepository> {
    pub user_repo: R,
    pub oauth_repo: O,
}

impl<R: UserRepository, O: OAuthAccountRepository> AccountLinkingService<R, O> {
    pub fn new(user_repo: R, oauth_repo: O) -> Self {
        Self {
            user_repo,
            oauth_repo,
        }
    }

    /// Handle OAuth login callback - main entry point for OAuth flow
    ///
    /// Logic:
    /// 1. Check if OAuth account already exists -> return existing user
    /// 2. Check if email matches existing user:
    ///    - If both emails verified + auto_link_verified_emails -> auto-link
    ///    - Otherwise -> return LinkRequired (manual linking needed)
    /// 3. Create new user if no match
    pub async fn handle_oauth_login(&self, login: OAuthLogin) -> UserResult<AccountLinkingResult> {
        let OAuthLogin {
            provider,
            user_info,
            access_token,
            refresh_token,
            expires_in,
            auto_link_verified_emails,
        } = login;

        // Check if OAuth account already exists
        if let Some(existing_account) = self
            .oauth_repo
            .find_by_provider_and_user_id(provider.as_str(), &user_info.provider_user_id)
            .await?
        {
            let user = self
                .user_repo
                .get_by_id(existing_account.user_id)
                .await?
                .ok_or(UserError::NotFound(existing_account.user_id))?;

            // Update tokens if provided
            if access_token.is_some() || refresh_token.is_some() {
                let token_expires_at =
                    expires_in.map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));
                self.oauth_repo
                    .update_tokens(
                        existing_account.id,
                        access_token.as_deref(),
                        refresh_token.as_deref(),
                        token_expires_at,
                    )
                    .await?;
            }

            return Ok(AccountLinkingResult::ExistingUser(user));
        }

        // Check if user with this email already exists
        if let Some(email) = &user_info.email
            && let Some(existing_user) = self.user_repo.get_by_email(email).await?
        {
            // Auto-link ONLY when the provider attests the email is verified
            // AND the local account's email is verified; anything less allows
            // account takeover via an attacker-controlled provider account.
            if auto_link_verified_emails && user_info.email_verified && existing_user.email_verified
            {
                self.link_oauth_to_user(
                    existing_user.id,
                    provider,
                    &user_info,
                    access_token,
                    refresh_token,
                    expires_in,
                )
                .await?;

                return Ok(AccountLinkingResult::ExistingUser(existing_user));
            } else {
                // Manual linking required
                return Ok(AccountLinkingResult::LinkRequired {
                    existing_user_id: existing_user.id,
                    provider_data: user_info,
                });
            }
        }

        // Create new user from OAuth data
        let new_user = self
            .create_user_from_oauth(
                provider,
                &user_info,
                access_token,
                refresh_token,
                expires_in,
            )
            .await?;

        Ok(AccountLinkingResult::NewUser(new_user))
    }

    /// Link OAuth account to an existing user
    pub async fn link_oauth_to_user(
        &self,
        user_id: Uuid,
        provider: Provider,
        user_info: &OAuthUserInfo,
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    ) -> UserResult<()> {
        // Check if this provider is already linked to this user
        let existing_link = self
            .oauth_repo
            .find_by_user_id_and_provider(user_id, provider.as_str())
            .await?;

        if existing_link.is_some() {
            return Err(UserError::OAuthAlreadyLinked(provider));
        }

        let token_expires_at =
            expires_in.map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

        self.oauth_repo
            .create(CreateOAuthAccountParams {
                user_id,
                provider: provider.as_str(),
                provider_user_id: &user_info.provider_user_id,
                provider_username: user_info.username.as_deref(),
                email: user_info.email.as_deref(),
                display_name: user_info.name.as_deref(),
                avatar_url: user_info.avatar_url.as_deref(),
                access_token: access_token.as_deref(),
                refresh_token: refresh_token.as_deref(),
                token_expires_at,
                scopes: None,
                raw_user_data: Some(user_info.raw_data.clone()),
            })
            .await?;

        Ok(())
    }

    /// Create a new user from OAuth data
    async fn create_user_from_oauth(
        &self,
        provider: Provider,
        user_info: &OAuthUserInfo,
        access_token: Option<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    ) -> UserResult<User> {
        let email = user_info.email.as_ref().ok_or_else(|| {
            UserError::OAuth(format!(
                "{provider} did not provide an email address for the new account"
            ))
        })?;

        let name = user_info
            .name
            .clone()
            .unwrap_or_else(|| email.split('@').next().unwrap_or("User").to_string());

        // Create user with random password (OAuth users don't use password login)
        let password_hash = password::hash_password(Uuid::new_v4().to_string()).await?;

        let mut user = User::new(email.clone(), name, password_hash, vec![Role::User]);

        // Mark email as verified if OAuth provider says so
        user.email_verified = user_info.email_verified;

        // Set avatar from OAuth
        if user_info.avatar_url.is_some() {
            user.avatar_url = user_info.avatar_url.clone();
        }

        let user = self.user_repo.create(user).await?;

        // Link OAuth account
        let token_expires_at =
            expires_in.map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

        self.oauth_repo
            .create(CreateOAuthAccountParams {
                user_id: user.id,
                provider: provider.as_str(),
                provider_user_id: &user_info.provider_user_id,
                provider_username: user_info.username.as_deref(),
                email: user_info.email.as_deref(),
                display_name: user_info.name.as_deref(),
                avatar_url: user_info.avatar_url.as_deref(),
                access_token: access_token.as_deref(),
                refresh_token: refresh_token.as_deref(),
                token_expires_at,
                scopes: None,
                raw_user_data: Some(user_info.raw_data.clone()),
            })
            .await?;

        Ok(user)
    }

    /// Unlink OAuth account from user
    ///
    /// Safety: Prevents unlinking the only OAuth account if user has no password
    pub async fn unlink_oauth(&self, user_id: Uuid, provider: Provider) -> UserResult<bool> {
        let user_oauth_accounts = self.oauth_repo.find_by_user_id(user_id).await?;

        // Prevent unlinking if this is the only OAuth account and user has no password
        if user_oauth_accounts.len() == 1 {
            let user = self
                .user_repo
                .get_by_id(user_id)
                .await?
                .ok_or(UserError::NotFound(user_id))?;

            // Check if user has a real password (not the random OAuth password)
            // A real password would have been set via password reset or account creation
            if user.password_hash.is_empty() || user.password_hash == "oauth_only" {
                return Err(UserError::CannotUnlinkOnlyAuthMethod);
            }
        }

        self.oauth_repo
            .delete_by_user_and_provider(user_id, provider.as_str())
            .await
    }

    /// Get all OAuth accounts for a user
    pub async fn get_user_oauth_accounts(
        &self,
        user_id: Uuid,
    ) -> UserResult<Vec<crate::oauth::OAuthAccount>> {
        self.oauth_repo.find_by_user_id(user_id).await
    }
}
