#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Corruption(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Corruption(message) => write!(f, "corruption: {message}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_and_displays() {
        let io_error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing file");
        let err = io_error.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("missing file"));
    }

    #[test]
    fn corruption_displays_message() {
        let err = Error::Corruption("bad crc".to_string());
        assert_eq!(err.to_string(), "corruption: bad crc");
    }
}
