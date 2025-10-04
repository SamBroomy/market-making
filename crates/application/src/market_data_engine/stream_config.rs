use infrastructure::{messaging::StreamProducer, persist::DataWriter};

/// Configuration for stream outputs (message queue + database)
#[derive(Clone)]
pub struct StreamConfig {
    pub producer: Option<StreamProducer>,
    pub writer: Option<DataWriter>,
}

impl StreamConfig {
    /// Create a new `StreamConfig` with both producer and writer
    pub fn new(producer: Option<StreamProducer>, writer: Option<DataWriter>) -> Self {
        Self { producer, writer }
    }

    /// Create a `StreamConfig` with only writer (no message queue)
    pub fn writer_only(writer: Option<DataWriter>) -> Self {
        Self {
            producer: None,
            writer,
        }
    }

    /// Create a `StreamConfig` with only producer (no database)
    pub fn producer_only(producer: Option<StreamProducer>) -> Self {
        Self {
            producer,
            writer: None,
        }
    }

    /// Split into individual components
    pub fn split(self) -> (Option<StreamProducer>, Option<DataWriter>) {
        (self.producer, self.writer)
    }
}
