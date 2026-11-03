//! The rotating app log: `logs/app.log` in the user data directory.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const LOG_LIMIT: u64 = 5 * 1024 * 1024;
static LOG_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn log_dir() -> PathBuf {
    crate::user_data::user_data_dir("codex-gpui").join("logs")
}

pub(crate) fn write(message: &str) {
    let _guard = LOG_LOCK.lock().unwrap();
    let result = (|| -> io::Result<()> {
        let dir = log_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join("app.log");
        if fs::metadata(&path).is_ok_and(|meta| meta.len() >= LOG_LIMIT) {
            let oldest = dir.join("app.log.2");
            let previous = dir.join("app.log.1");
            if let Err(error) = fs::remove_file(&oldest)
                && error.kind() != io::ErrorKind::NotFound
            {
                return Err(error);
            }
            if previous.exists() {
                fs::rename(&previous, &oldest)?;
            }
            fs::rename(&path, &previous)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writeln!(file, "{timestamp} {message}")
    })();
    if let Err(error) = result {
        std::eprintln!("codex-gpui: cannot write log: {error}");
    }
}

pub(crate) fn export(directory: &Path) -> io::Result<PathBuf> {
    let _guard = LOG_LOCK.lock().unwrap();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let output = directory.join(format!("codex-gpui-logs-{timestamp}.zip"));
    let dir = log_dir();
    let mut archive = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)?;
    if let Err(error) = write_zip(&mut archive, &dir) {
        let _ = fs::remove_file(&output);
        return Err(error);
    }
    Ok(output)
}

fn write_zip(output: &mut impl Write, directory: &Path) -> io::Result<()> {
    let mut central = Vec::new();
    let mut offset = 0u32;
    let mut entry_count = 0u16;
    for name in ["app.log.2", "app.log.1", "app.log"] {
        let path = directory.join(name);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let (size, crc) = file_crc32(&mut file)?;
        drop(file);
        let name_bytes = name.as_bytes();
        output.write_all(&0x0403_4b50u32.to_le_bytes())?;
        output.write_all(&20u16.to_le_bytes())?;
        output.write_all(&0u16.to_le_bytes())?;
        output.write_all(&0u16.to_le_bytes())?;
        output.write_all(&0u16.to_le_bytes())?;
        output.write_all(&33u16.to_le_bytes())?;
        output.write_all(&crc.to_le_bytes())?;
        output.write_all(&size.to_le_bytes())?;
        output.write_all(&size.to_le_bytes())?;
        output.write_all(&(name_bytes.len() as u16).to_le_bytes())?;
        output.write_all(&0u16.to_le_bytes())?;
        output.write_all(name_bytes)?;
        let copied = io::copy(&mut File::open(&path)?, output)?;
        if copied != u64::from(size) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "log changed during export",
            ));
        }
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&[0; 4]);
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&33u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
        let entry_size = 30u32
            .checked_add(name_bytes.len() as u32)
            .and_then(|length| length.checked_add(size))
            .ok_or_else(|| io::Error::other("log archive is too large"))?;
        offset = offset
            .checked_add(entry_size)
            .ok_or_else(|| io::Error::other("log archive is too large"))?;
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(|| io::Error::other("too many log files"))?;
    }
    let central_offset = offset;
    output.write_all(&central)?;
    let central_size = u32::try_from(central.len()).map_err(io::Error::other)?;
    output.write_all(&0x0605_4b50u32.to_le_bytes())?;
    output.write_all(&[0; 4])?;
    output.write_all(&entry_count.to_le_bytes())?;
    output.write_all(&entry_count.to_le_bytes())?;
    output.write_all(&central_size.to_le_bytes())?;
    output.write_all(&central_offset.to_le_bytes())?;
    output.write_all(&0u16.to_le_bytes())
}

fn file_crc32(file: &mut File) -> io::Result<(u32, u32)> {
    let mut buffer = [0; 8192];
    let mut size = 0u32;
    let mut crc = !0u32;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok((size, !crc));
        }
        size = size
            .checked_add(read as u32)
            .ok_or_else(|| io::Error::other("log archive is too large"))?;
        crc = update_crc32(crc, &buffer[..read]);
    }
}

fn update_crc32(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    crc
}

#[cfg(test)]
fn crc32(bytes: &[u8]) -> u32 {
    !update_crc32(!0u32, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_valid_stored_zip() {
        let content = b"entry\n";
        let directory = std::env::temp_dir().join(format!(
            "codex-gpui-log-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("app.log"), content).unwrap();
        let mut zip = Vec::new();
        write_zip(&mut zip, &directory).unwrap();
        assert_eq!(&zip[0..4], b"PK\x03\x04");
        assert_eq!(&zip[14..18], &crc32(content).to_le_bytes());
        assert_eq!(&zip[30..37], b"app.log");
        assert_eq!(&zip[37..43], content);
        assert_eq!(&zip[43..47], b"PK\x01\x02");
        assert_eq!(&zip[96..100], b"PK\x05\x06");
        assert_eq!(u32::from_le_bytes(zip[112..116].try_into().unwrap()), 43);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        fs::remove_dir_all(directory).unwrap();
    }
}
