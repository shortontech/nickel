use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

pub struct ThreadLease {
    path: PathBuf,
}

impl ThreadLease {
    pub fn acquire(thread: &str) -> Result<Self, String> {
        let runtime = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("nickel-codex-leases");
        fs::create_dir_all(&runtime).map_err(|error| error.to_string())?;
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in thread.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let path = runtime.join(format!("{hash:016x}"));
        #[cfg(target_os = "linux")]
        if let Ok(owner) = fs::read_to_string(&path)
            && owner
                .trim()
                .parse::<u32>()
                .ok()
                .is_none_or(|pid| !std::path::Path::new("/proc").join(pid.to_string()).exists())
        {
            let _ = fs::remove_file(&path);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                format!("conversation already has an active Nickel writer ({error})")
            })?;
        writeln!(file, "{}", std::process::id()).map_err(|error| error.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for ThreadLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_one_writer_and_releases_on_drop() {
        let id = format!("lease-test-{}", std::process::id());
        let lease = ThreadLease::acquire(&id).unwrap();
        assert!(ThreadLease::acquire(&id).is_err());
        drop(lease);
        assert!(ThreadLease::acquire(&id).is_ok());
    }
}
