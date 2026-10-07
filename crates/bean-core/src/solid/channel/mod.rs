//! Channel Adapter Pattern - Open/Closed Principle (OCP)
//!
//! This module provides a flexible channel system that supports
//! multiple communication channels (Telegram, Web, CLI) without
//! modifying existing code when adding new channels.

use std::sync::Arc;

use crate::router::{Router, RouterError};
use crate::run_io::Decision;
use bean_types::{Outbound, SessionId};

/// Event types that can be sent from channels
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    Message { session_id: SessionId, text: String },
    Confirm { confirm_id: String, decision: Decision },
    Cancel { session_id: SessionId },
    Sync,
}

/// Base trait for all channel adapters
pub trait ChannelAdapter: Send + Sync + 'static {
    /// Get the channel name
    fn name(&self) -> &'static str;

    /// Get the channel description
    fn description(&self) -> &'static str {
        self.name()
    }

    /// Start the channel adapter
    async fn start(&self, router: Arc<Router>) -> Result<(), ChannelError>;

    /// Stop the channel adapter
    async fn stop(&self) -> Result<(), ChannelError>;

    /// Check if the channel is running
    fn is_running(&self) -> bool;

    /// Send a message through this channel
    async fn send(&self, chat_id: &str, message: Outbound) -> Result<(), ChannelError>;

    /// Get the configuration for this channel
    fn config(&self) -> &ChannelConfig;
}

/// Configuration for a channel adapter
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    pub name: String,
    pub enabled: bool,
    pub rate_limit: Option<RateLimitConfig>,
    pub security: SecurityConfig,
}

impl ChannelConfig {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            enabled: true,
            rate_limit: None,
            security: SecurityConfig::default(),
        }
    }
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self::new("default")
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
    pub burst_size: u32,
}

impl RateLimitConfig {
    pub fn new(requests_per_minute: u32, burst_size: u32) -> Self {
        Self {
            requests_per_minute,
            burst_size,
        }
    }
}

/// Security configuration for a channel
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    pub allowlist_enabled: bool,
    pub allowed_users: Vec<String>,
    pub max_message_size: usize,
    pub require_confirmation: bool,
}

impl SecurityConfig {
    pub fn new() -> Self {
        Self {
            allowlist_enabled: false,
            allowed_users: Vec::new(),
            max_message_size: 4096,
            require_confirmation: true,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Error types for channel operations
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("Channel not running: {0}")]
    NotRunning(&'static str),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Security violation: {0}")]
    SecurityViolation(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),
}

/// Factory trait for creating channel adapters
pub trait ChannelAdapterFactory: Send + Sync + 'static {
    fn channel_name(&self) -> &'static str;
    fn create(&self) -> Box<dyn ChannelAdapter>;
}

/// Registry for channel adapters
pub struct ChannelRegistry {
    factories: Vec<Box<dyn ChannelAdapterFactory>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            factories: Vec::new(),
        }
    }

    /// Register a new channel factory - OCP: Open for extension
    pub fn register<F>(&mut self, factory: F) -> &mut Self
    where
        F: ChannelAdapterFactory + 'static,
    {
        self.factories.push(Box::new(factory));
        self
    }

    /// Get all registered channel names
    pub fn list_channels(&self) -> Vec<&'static str> {
        self.factories
            .iter()
            .map(|f| f.channel_name())
            .collect()
    }

    /// Create a channel adapter by name
    pub fn create(&self, name: &str) -> Option<Box<dyn ChannelAdapter>> {
        self.factories
            .iter()
            .find(|f| f.channel_name() == name)
            .map(|f| f.create())
    }

    /// Create all registered channel adapters
    pub fn create_all(&self) -> Vec<Box<dyn ChannelAdapter>> {
        self.factories
            .iter()
            .map(|f| f.create())
            .collect()
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Channel event handler trait
pub trait ChannelEventHandler: Send + Sync + 'static {
    fn handle_event(&self, event: ChannelEvent);
}

/// Event dispatcher for channel events
pub struct ChannelEventDispatcher {
    handlers: Vec<Arc<dyn ChannelEventHandler>>,
}

impl ChannelEventDispatcher {
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    pub fn register_handler(&mut self, handler: Arc<dyn ChannelEventHandler>) {
        self.handlers.push(handler);
    }

    pub fn dispatch(&self, event: ChannelEvent) {
        for handler in &self.handlers {
            handler.handle_event(event.clone());
        }
    }
}

impl Default for ChannelEventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Telegram channel adapter
pub struct TelegramChannel {
    config: ChannelConfig,
    running: std::sync::atomic::AtomicBool,
}

impl TelegramChannel {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ChannelAdapter for TelegramChannel {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn start(&self, _router: Arc<Router>) -> Result<(), ChannelError> {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn send(&self, _chat_id: &str, _message: Outbound) -> Result<(), ChannelError> {
        // Implementation would use teloxide
        Ok(())
    }

    fn config(&self) -> &ChannelConfig {
        &self.config
    }
}

/// Web channel adapter (HTTP/WebSocket)
pub struct WebChannel {
    config: ChannelConfig,
    running: std::sync::atomic::AtomicBool,
}

impl WebChannel {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ChannelAdapter for WebChannel {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn start(&self, _router: Arc<Router>) -> Result<(), ChannelError> {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn send(&self, _chat_id: &str, _message: Outbound) -> Result<(), ChannelError> {
        // Implementation would use axum
        Ok(())
    }

    fn config(&self) -> &ChannelConfig {
        &self.config
    }
}

/// CLI channel adapter
pub struct CliChannel {
    config: ChannelConfig,
    running: std::sync::atomic::AtomicBool,
}

impl CliChannel {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl ChannelAdapter for CliChannel {
    fn name(&self) -> &'static str {
        "cli"
    }

    async fn start(&self, _router: Arc<Router>) -> Result<(), ChannelError> {
        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn stop(&self) -> Result<(), ChannelError> {
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn send(&self, _chat_id: &str, _message: Outbound) -> Result<(), ChannelError> {
        // Implementation would use crossterm/rustyline
        Ok(())
    }

    fn config(&self) -> &ChannelConfig {
        &self.config
    }
}
