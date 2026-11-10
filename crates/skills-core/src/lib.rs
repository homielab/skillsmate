//! Core logic for SkillsMate, a desktop GUI for the skills.sh ecosystem.
//!
//! This crate has no UI dependencies so it can be tested on its own. It mirrors
//! the on-disk behaviour of the official `skills` CLI (vercel-labs/skills):
//!
//! - Skills live in a canonical directory, `<base>/.agents/skills/<name>`, where
//!   `<base>` is the project root or the user's home directory.
//! - Agents that read `.agents/skills` directly ("universal" agents) need nothing
//!   more; every other agent gets a symlink (a junction on Windows) or a copy in
//!   its own skills directory.
//! - Global installs are recorded in `~/.agents/.skill-lock.json` (v3), project
//!   installs in `<project>/skills-lock.json` (v1).

#![forbid(unsafe_code)]

pub mod agents;
pub mod app_update;
pub mod directory;
pub mod env;
pub mod error;
pub mod files;
pub mod health;
pub mod lock;
pub mod manage;
pub mod markdown;
pub mod recommend;
pub mod sanitize;
pub mod scan;
pub mod search;
pub mod skill_md;
pub mod stats;

pub use agents::{Agent, AgentId};
pub use env::{Env, Scope};
pub use error::{Error, Result};
