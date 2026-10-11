//! Playlist readers and conversion, isolated from account-owning source plugins.
pub mod errors;
pub mod models;
pub mod providers;
pub mod sources;
mod http;
pub mod broker;
pub mod matching;
pub mod service;
pub mod identify;
pub mod importer {
    pub async fn preview_public_playlist(url: &str) -> Result<crate::models::ImportPlaylist, crate::errors::PlaylistImportError> {
        crate::providers::fetch_playlist(crate::providers::detect_provider(url)?).await
    }
}
pub const QBZ_PROXY_BASE: &str = "https://qbz-api-proxy.blitzkriegfc.workers.dev";
