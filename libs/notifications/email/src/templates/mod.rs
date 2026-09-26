//! Email template management with Handlebars
//!
//! This module provides:
//! - `TemplateEngine`: Handlebars-based template rendering
//! - `TemplateStore` trait and `InMemoryTemplateStore` for template storage
//! - Default templates for common email types

use eyre::Result;
use handlebars::Handlebars;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Errors from template registration and rendering.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// Registering a template string with Handlebars failed.
    #[error("template registration failed: {0}")]
    Registration(#[from] Box<handlebars::TemplateError>),
    /// Rendering a registered template failed.
    #[error("template render failed: {0}")]
    Render(#[from] handlebars::RenderError),
    /// No template registered under the requested name.
    #[error("template not found: {0}")]
    NotFound(String),
}

/// Rendered template result
#[derive(Debug, Clone)]
pub struct RenderedTemplate {
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
}

/// Template store trait
#[async_trait::async_trait]
pub trait TemplateStore: Send + Sync {
    /// Get a template by name
    async fn get(&self, name: &str) -> Result<Option<EmailTemplate>>;

    /// Store a template
    async fn set(&self, template: EmailTemplate) -> Result<()>;

    /// List all template names
    async fn list(&self) -> Result<Vec<String>>;
}

/// Email template definition
#[derive(Clone, Debug)]
pub struct EmailTemplate {
    pub name: String,
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
}

impl EmailTemplate {
    /// Render the template with the given data using simple replacement
    /// For backward compatibility with existing code
    pub fn render(&self, data: &Value) -> Result<RenderedTemplate> {
        let subject = render_string(&self.subject, data)?;
        let body_text = self
            .body_text
            .as_ref()
            .map(|t| render_string(t, data))
            .transpose()?;
        let body_html = self
            .body_html
            .as_ref()
            .map(|t| render_string(t, data))
            .transpose()?;

        Ok(RenderedTemplate {
            subject,
            body_text,
            body_html,
        })
    }
}

/// Simple template variable replacement (legacy)
fn render_string(template: &str, data: &Value) -> Result<String> {
    let mut result = template.to_string();

    if let Value::Object(map) = data {
        for (key, value) in map {
            let placeholder = format!("{{{{{key}}}}}");
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                _ => serde_json::to_string(value).unwrap_or_default(),
            };
            result = result.replace(&placeholder, &replacement);
        }
    }

    Ok(result)
}

/// Handlebars-based template engine
///
/// Supports:
/// - Variables: `{{name}}`
/// - Conditionals: `{{#if condition}}...{{/if}}`
/// - Loops: `{{#each items}}...{{/each}}`
/// - HTML escaping: `{{{unescaped}}}` for raw HTML
pub struct TemplateEngine {
    handlebars: Handlebars<'static>,
    templates: HashMap<String, EmailTemplate>,
}

impl TemplateEngine {
    /// Create a new TemplateEngine with default templates
    pub fn new() -> Result<Self, TemplateError> {
        let mut engine = Self {
            handlebars: Handlebars::new(),
            templates: HashMap::new(),
        };
        engine.register_defaults()?;
        Ok(engine)
    }

    /// Register a template
    pub fn register(&mut self, template: EmailTemplate) -> Result<(), TemplateError> {
        self.handlebars
            .register_template_string(&format!("{}_subject", template.name), &template.subject)
            .map_err(Box::new)?;
        if let Some(text) = &template.body_text {
            self.handlebars
                .register_template_string(&format!("{}_text", template.name), text)
                .map_err(Box::new)?;
        }
        if let Some(html) = &template.body_html {
            self.handlebars
                .register_template_string(&format!("{}_html", template.name), html)
                .map_err(Box::new)?;
        }
        self.templates.insert(template.name.clone(), template);
        Ok(())
    }

    /// Render a template by name
    pub fn render(&self, name: &str, data: &Value) -> Result<RenderedTemplate, TemplateError> {
        let template = self
            .templates
            .get(name)
            .ok_or_else(|| TemplateError::NotFound(name.to_string()))?;

        let subject = self.handlebars.render(&format!("{name}_subject"), data)?;

        let body_text = if template.body_text.is_some() {
            Some(self.handlebars.render(&format!("{name}_text"), data)?)
        } else {
            None
        };

        let body_html = if template.body_html.is_some() {
            Some(self.handlebars.render(&format!("{name}_html"), data)?)
        } else {
            None
        };

        Ok(RenderedTemplate {
            subject,
            body_text,
            body_html,
        })
    }

    /// Check if a template exists
    pub fn has_template(&self, name: &str) -> bool {
        self.templates.contains_key(name)
    }

    /// List all registered templates
    pub fn list_templates(&self) -> Vec<&str> {
        self.templates.keys().map(|s| s.as_str()).collect()
    }

    /// Register default email templates
    fn register_defaults(&mut self) -> Result<(), TemplateError> {
        // Welcome email
        self.register(EmailTemplate {
            name: "welcome".to_string(),
            subject: "Welcome to {{app_name}}, {{name}}!".to_string(),
            body_text: Some(
                r#"Hello {{name}},

Welcome to {{app_name}}!

We're excited to have you on board.

Best regards,
The {{app_name}} Team"#
                    .to_string(),
            ),
            body_html: Some(
                r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <h1 style="color: #2563eb;">Welcome, {{name}}!</h1>
    <p>Thank you for joining <strong>{{app_name}}</strong>.</p>
    <p>We're excited to have you on board.</p>
    <p>Best regards,<br>The {{app_name}} Team</p>
</body>
</html>"#
                    .to_string(),
            ),
        })?;

        // Email verification
        self.register(EmailTemplate {
            name: "verification".to_string(),
            subject: "Verify your email for {{app_name}}".to_string(),
            body_text: Some(
                r#"Hello {{name}},

Please verify your email address by clicking the link below:

{{verification_link}}

This link will expire in {{expiry_hours}} hours.

If you didn't create an account, you can safely ignore this email.

Best regards,
The {{app_name}} Team"#
                    .to_string(),
            ),
            body_html: Some(
                r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <h1 style="color: #2563eb;">Verify Your Email</h1>
    <p>Hello {{name}},</p>
    <p>Please verify your email address by clicking the button below:</p>
    <p style="text-align: center; margin: 30px 0;">
        <a href="{{verification_link}}"
           style="background-color: #2563eb; color: white; padding: 12px 24px; text-decoration: none; border-radius: 6px; display: inline-block;">
            Verify Email
        </a>
    </p>
    <p style="color: #666; font-size: 14px;">This link will expire in {{expiry_hours}} hours.</p>
    <p style="color: #666; font-size: 14px;">If you didn't create an account, you can safely ignore this email.</p>
    <p>Best regards,<br>The {{app_name}} Team</p>
</body>
</html>"#
                    .to_string(),
            ),
        })?;

        // Password reset
        self.register(EmailTemplate {
            name: "password_reset".to_string(),
            subject: "Password Reset Request".to_string(),
            body_text: Some(
                r#"Hello {{name}},

We received a request to reset your password.

Click the link below to reset your password:

{{reset_link}}

This link will expire in {{expiry_hours}} hours.

If you didn't request this, please ignore this email. Your password will remain unchanged.

Best regards,
The {{app_name}} Team"#
                    .to_string(),
            ),
            body_html: Some(
                r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <h1 style="color: #2563eb;">Password Reset</h1>
    <p>Hello {{name}},</p>
    <p>We received a request to reset your password.</p>
    <p style="text-align: center; margin: 30px 0;">
        <a href="{{reset_link}}"
           style="background-color: #dc2626; color: white; padding: 12px 24px; text-decoration: none; border-radius: 6px; display: inline-block;">
            Reset Password
        </a>
    </p>
    <p style="color: #666; font-size: 14px;">This link will expire in {{expiry_hours}} hours.</p>
    <p style="color: #666; font-size: 14px;">If you didn't request this, please ignore this email. Your password will remain unchanged.</p>
    <p>Best regards,<br>The {{app_name}} Team</p>
</body>
</html>"#
                    .to_string(),
            ),
        })?;

        // Password changed confirmation
        self.register(EmailTemplate {
            name: "password_changed".to_string(),
            subject: "Your password has been changed".to_string(),
            body_text: Some(
                r#"Hello {{name}},

Your password has been successfully changed.

If you did not make this change, please contact support immediately.

Best regards,
The {{app_name}} Team"#
                    .to_string(),
            ),
            body_html: Some(
                r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <h1 style="color: #2563eb;">Password Changed</h1>
    <p>Hello {{name}},</p>
    <p>Your password has been successfully changed.</p>
    <p style="color: #dc2626; font-weight: bold;">If you did not make this change, please contact support immediately.</p>
    <p>Best regards,<br>The {{app_name}} Team</p>
</body>
</html>"#
                    .to_string(),
            ),
        })?;

        Ok(())
    }
}

/// In-memory template store (async, for backward compatibility)
pub struct InMemoryTemplateStore {
    templates: Arc<RwLock<HashMap<String, EmailTemplate>>>,
}

impl InMemoryTemplateStore {
    pub fn new() -> Self {
        Self {
            templates: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create with default templates (synchronously initialized)
    pub fn with_defaults() -> Self {
        let mut templates = HashMap::new();

        templates.insert(
            "welcome".to_string(),
            EmailTemplate {
                name: "welcome".to_string(),
                subject: "Welcome to {{app_name}}, {{name}}!".to_string(),
                body_text: Some(
                    "Hello {{name}},\n\nWelcome to {{app_name}}!\n\nBest regards,\nThe Team"
                        .to_string(),
                ),
                body_html: Some(
                    r#"<h1>Welcome, {{name}}!</h1>
<p>Thank you for joining {{app_name}}.</p>
<p>Best regards,<br>The Team</p>"#
                        .to_string(),
                ),
            },
        );

        templates.insert(
            "password_reset".to_string(),
            EmailTemplate {
                name: "password_reset".to_string(),
                subject: "Password Reset Request".to_string(),
                body_text: Some("Hello {{name}},\n\nClick here to reset your password: {{reset_link}}\n\nThis link expires in {{expiry_hours}} hours.".to_string()),
                body_html: Some(r#"<h1>Password Reset</h1>
<p>Hello {{name}},</p>
<p>Click <a href="{{reset_link}}">here</a> to reset your password.</p>
<p>This link expires in {{expiry_hours}} hours.</p>"#.to_string()),
            },
        );

        Self {
            templates: Arc::new(RwLock::new(templates)),
        }
    }
}

impl Default for InMemoryTemplateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl TemplateStore for InMemoryTemplateStore {
    async fn get(&self, name: &str) -> Result<Option<EmailTemplate>> {
        let guard = self.templates.read().await;
        Ok(guard.get(name).cloned())
    }

    async fn set(&self, template: EmailTemplate) -> Result<()> {
        let mut guard = self.templates.write().await;
        guard.insert(template.name.clone(), template);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<String>> {
        let guard = self.templates.read().await;
        Ok(guard.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_engine_creation() {
        let engine = TemplateEngine::new().unwrap();
        assert!(engine.has_template("welcome"));
        assert!(engine.has_template("password_reset"));
        assert!(engine.has_template("verification"));
    }

    #[test]
    fn test_template_rendering() {
        let engine = TemplateEngine::new().unwrap();

        let data = serde_json::json!({
            "name": "John",
            "app_name": "TestApp"
        });

        let rendered = engine.render("welcome", &data).unwrap();

        assert!(rendered.subject.contains("John"));
        assert!(rendered.subject.contains("TestApp"));
        assert!(rendered.body_text.unwrap().contains("John"));
        assert!(rendered.body_html.unwrap().contains("John"));
    }

    #[test]
    fn test_custom_template() {
        let mut engine = TemplateEngine::new().unwrap();

        let template = EmailTemplate {
            name: "custom".to_string(),
            subject: "Custom: {{title}}".to_string(),
            body_text: Some("{{content}}".to_string()),
            body_html: None,
        };

        engine.register(template).unwrap();

        let data = serde_json::json!({
            "title": "Test",
            "content": "Hello World"
        });

        let rendered = engine.render("custom", &data).unwrap();
        assert_eq!(rendered.subject, "Custom: Test");
        assert_eq!(rendered.body_text.unwrap(), "Hello World");
    }
}
