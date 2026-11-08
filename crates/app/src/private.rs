//! Owner-only file modes and the executable bit: Unix permission bits, a no-op on Windows,
//! where the user profile's ACLs already keep other users out.
use std::{fs, io, path::Path};

/// Sets the Unix permission bits of `path`; on Windows it does nothing.
pub fn restrict(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Whether `metadata` describes a file the OS can execute; Windows decides by extension, so any file counts.
pub fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}
