use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error::Message(message.into())
    }
}

// Commands reject with a plain string so the frontend can show it directly.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<speech_connector::Error> for Error {
    fn from(value: speech_connector::Error) -> Self {
        Self::msg(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_a_plain_message() {
        let err = Error::msg("deck not found: x");
        assert_eq!(serde_json::to_value(&err).unwrap(), "deck not found: x");
        let io: Error = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(serde_json::to_value(&io).unwrap(), "gone");
    }
}
