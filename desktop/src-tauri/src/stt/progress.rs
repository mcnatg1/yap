use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeProgressEvent {
    pub path: String,
    pub index: usize,
    pub total: usize,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    pub message: String,
}

#[derive(Clone)]
pub struct ProgressSink {
    callback: Arc<dyn Fn(TranscribeProgressEvent) + Send + Sync>,
}

impl ProgressSink {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(TranscribeProgressEvent) + Send + Sync + 'static,
    {
        Self { callback: Arc::new(callback) }
    }

    pub fn emit(&self, event: TranscribeProgressEvent) {
        (self.callback)(event);
    }
}

#[derive(Clone)]
pub struct ProgressReporter {
    sink: ProgressSink,
    path: String,
    index: usize,
    total: usize,
}

impl ProgressReporter {
    pub fn new(sink: ProgressSink, path: String, index: usize, total: usize) -> Self {
        Self { sink, path, index, total }
    }

    pub fn emit(&self, phase: &str, percent: Option<u8>, message: &str) {
        self.sink.emit(TranscribeProgressEvent {
            path: self.path.clone(),
            index: self.index,
            total: self.total,
            phase: phase.to_string(),
            percent,
            message: message.to_string(),
        });
    }
}
