//! Portal uses the shared bounded node backend directly.
use anyhow::Result;
use minos::node::LocalNode;
pub struct PortalNode(LocalNode);
impl PortalNode {
    pub fn new(socket_path: String, network_magic: u64) -> Self {
        Self(LocalNode::new(socket_path, network_magic))
    }
    pub fn with_timeout(mut self, duration: std::time::Duration) -> Self {
        self.0 = self.0.with_timeout(duration);
        self
    }
    pub async fn session(&self) -> Result<minos::node::NodeSession> {
        self.0.session().await
    }
}
