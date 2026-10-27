//! Minecraft version install, offline / authlib-injector login, and launch.

pub mod auth;
pub mod download;
pub mod java;
pub mod launch;
pub mod meta;
pub mod model;

pub use auth::{ensure_authlib, offline_account, yggdrasil_login, yggdrasil_refresh};
pub use download::{
    delete_version, http_client, install_version, installed_ids, list_versions, rename_version, Report,
};
pub use java::{discover_javas, pick_java};
pub use launch::{prepare_launch, spawn_launch};
pub use model::*;
