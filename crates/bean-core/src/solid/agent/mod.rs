//! Agent module - OOP refactoring following SOLID principles
//!
//! This module contains the core agent logic split into focused components:
//! - AgentContext: Manages context building and memory
//! - AgentPolicy: Handles security policies and permissions
//! - AgentLogger: Manages audit logging
//! - AgentExecutor: Executes the agent loop

pub mod core;

pub use core::{AgentContext, AgentPolicy, AgentLogger, AgentExecutor};
