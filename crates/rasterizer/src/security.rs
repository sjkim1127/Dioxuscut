//! Security sandbox policy for media asset loading.

use crate::backend::RasterError;
use std::path::{Path, PathBuf};

/// Media security sandbox policy controlling file access for video, audio, image, and Lottie assets.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MediaSecurityPolicy {
    /// Allow access to any local file or file:// URI that exists and can be resolved.
    #[default]
    Permissive,
    /// Strictly restrict media file access to designated root directories.
    Sandboxed {
        /// Canonicalized root paths permitted for file access.
        allowed_roots: Vec<PathBuf>,
    },
}

impl MediaSecurityPolicy {
    /// Create a permissive policy allowing any valid local path.
    pub fn permissive() -> Self {
        Self::Permissive
    }

    /// Create a sandboxed policy restricting media access to the specified directories.
    ///
    /// Paths that do not exist or cannot be canonicalized at creation time are ignored.
    pub fn sandboxed(roots: impl IntoIterator<Item = impl AsRef<Path>>) -> Self {
        let allowed_roots = roots
            .into_iter()
            .filter_map(|r| r.as_ref().canonicalize().ok())
            .collect();
        Self::Sandboxed { allowed_roots }
    }

    /// Add an allowed root directory to the sandbox policy.
    pub fn with_allowed_root(mut self, root: impl AsRef<Path>) -> Self {
        if let Ok(canonical) = root.as_ref().canonicalize() {
            match &mut self {
                Self::Permissive => {
                    self = Self::Sandboxed {
                        allowed_roots: vec![canonical],
                    };
                }
                Self::Sandboxed { allowed_roots } => {
                    if !allowed_roots.contains(&canonical) {
                        allowed_roots.push(canonical);
                    }
                }
            }
        }
        self
    }

    /// Validate and canonicalize a source path or `file://` URI.
    ///
    /// Returns the canonical `PathBuf` if permitted, or returns a `RasterError` if:
    /// - The source path is empty
    /// - It is a remote URL (http/https/data)
    /// - It does not exist on disk
    /// - It violates sandbox constraints (outside all allowed roots)
    pub fn validate_path(&self, src: &str) -> Result<PathBuf, RasterError> {
        let trimmed = src.trim();
        if trimmed.is_empty() {
            return Err(RasterError::MediaAsset {
                path: src.to_string(),
                reason: "source path is empty".into(),
            });
        }

        let raw_path = if let Some(path) = trimmed.strip_prefix("file://") {
            path
        } else if trimmed.contains("://") || trimmed.starts_with("data:") {
            return Err(RasterError::MediaAsset {
                path: src.to_string(),
                reason: "only local paths and file:// URIs are supported".into(),
            });
        } else {
            trimmed
        };

        let canonical =
            Path::new(raw_path)
                .canonicalize()
                .map_err(|err| RasterError::MediaAsset {
                    path: src.to_string(),
                    reason: err.to_string(),
                })?;

        match self {
            Self::Permissive => Ok(canonical),
            Self::Sandboxed { allowed_roots } => {
                let is_allowed = allowed_roots.iter().any(|root| canonical.starts_with(root));
                if is_allowed {
                    Ok(canonical)
                } else {
                    Err(RasterError::SecurityViolation {
                        path: src.to_string(),
                        reason: format!(
                            "path '{}' is outside allowed sandbox roots: {:?}",
                            canonical.display(),
                            allowed_roots
                        ),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejects_empty_and_remote_sources() {
        let policy = MediaSecurityPolicy::permissive();
        assert!(policy.validate_path("").is_err());
        assert!(policy.validate_path("   ").is_err());
        assert!(policy
            .validate_path("https://example.com/video.mp4")
            .is_err());
        assert!(policy
            .validate_path("data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==")
            .is_err());
    }

    #[test]
    fn test_permissive_allows_existing_files() {
        let policy = MediaSecurityPolicy::permissive();
        let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let validated = policy.validate_path(cargo_toml.to_str().unwrap()).unwrap();
        assert_eq!(validated, cargo_toml.canonicalize().unwrap());

        let file_uri = format!("file://{}", cargo_toml.display());
        let validated_uri = policy.validate_path(&file_uri).unwrap();
        assert_eq!(validated_uri, cargo_toml.canonicalize().unwrap());
    }

    #[test]
    fn test_sandboxed_allows_files_inside_root() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let policy = MediaSecurityPolicy::sandboxed([manifest_dir]);
        let cargo_toml = manifest_dir.join("Cargo.toml");
        let validated = policy.validate_path(cargo_toml.to_str().unwrap()).unwrap();
        assert_eq!(validated, cargo_toml.canonicalize().unwrap());
    }

    #[test]
    fn test_sandboxed_blocks_traversal_outside_root() {
        let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let policy = MediaSecurityPolicy::sandboxed([&src_dir]);

        // Cargo.toml is in the parent directory of src/
        let cargo_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let result = policy.validate_path(cargo_toml.to_str().unwrap());
        assert!(result.is_err());
        match result.unwrap_err() {
            RasterError::SecurityViolation { path, reason } => {
                assert!(path.contains("Cargo.toml"));
                assert!(reason.contains("outside allowed sandbox roots"));
            }
            other => panic!("expected SecurityViolation, got {other:?}"),
        }

        // Relative path traversal attack: src/../Cargo.toml
        let traversal = src_dir.join("../Cargo.toml");
        let result_trav = policy.validate_path(traversal.to_str().unwrap());
        assert!(result_trav.is_err());
    }
}
