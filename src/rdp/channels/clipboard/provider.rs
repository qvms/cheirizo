//! Local clipboard backend contract used by the CLIPRDR orchestrator.

use super::error::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAuthority {
    Provider,
    PossibleEcho,
}

#[derive(Debug, Clone)]
pub enum ClipboardProviderEvent {
    SelectionChanged {
        mime_types: Vec<String>,
        force: bool,
    },
    SelectionTransfer {
        serial: u32,
        mime_type: String,
    },
}

#[async_trait]
pub trait ClipboardProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports_file_transfer(&self) -> bool;
    fn requires_upfront_data(&self) -> bool;
    async fn announce_formats(&self, mime_types: Vec<String>) -> Result<()>;
    async fn read_data(&self, mime_type: &str) -> Result<Vec<u8>>;
    async fn provide_data(&self, mime_type: &str, data: Vec<u8>) -> Result<()>;
    async fn complete_transfer(
        &self,
        serial: u32,
        mime_type: &str,
        data: Vec<u8>,
        success: bool,
    ) -> Result<()>;
    fn subscribe(&self) -> mpsc::UnboundedReceiver<ClipboardProviderEvent>;
    async fn health_check(&self) -> Result<()>;
    async fn write_text(&self, text: &str) -> Result<()>;
    async fn shutdown(&self);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_preserves_transfer_identity() {
        let e = ClipboardProviderEvent::SelectionTransfer {
            serial: 7,
            mime_type: "text/plain".into(),
        };
        assert!(matches!(
            e,
            ClipboardProviderEvent::SelectionTransfer { serial: 7, .. }
        ));
    }
}
