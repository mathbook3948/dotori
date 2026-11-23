use std::fmt;
use std::sync::Arc;

use crate::channel::AppWindow;
use crate::channel::Channel;
use crate::llm::Model;
use crate::record::RecordStore;
use crate::sub_session::SubSessionRunner;
use crate::tool::Toolbox;

/// Everything the agent talks to. The app crate builds it from concrete connectors.
pub struct Ports {
    pub app: Arc<dyn AppWindow>,
    pub channels: Vec<Arc<dyn Channel>>,
    pub model: Arc<dyn Model>,
    pub toolbox: Arc<dyn Toolbox>,
    pub sub_sessions: Arc<dyn SubSessionRunner>,
    pub records: Arc<dyn RecordStore>,
}

impl fmt::Debug for Ports {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ports").finish_non_exhaustive()
    }
}
