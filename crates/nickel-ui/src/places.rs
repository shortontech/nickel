use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

use crate::model::Application;

pub fn applications() -> Vec<Application> {
    let home = home_directory();
    let candidates = [
        ("Home", home.clone()),
        ("Desktop", home.join("Desktop")),
        ("Documents", home.join("Documents")),
        ("Downloads", home.join("Downloads")),
        ("Music", home.join("Music")),
        ("Pictures", home.join("Pictures")),
        ("Videos", home.join("Videos")),
    ];
    let file_manager = nickel_file_executable();
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|(_, path)| path.is_dir())
        .filter(|(_, path)| seen.insert(normalized_path(path)))
        .map(|(name, path)| {
            let path_text = path.to_string_lossy().into_owned();
            Application::new(
                format!("place:{}", normalized_path(&path)),
                name.to_owned(),
                Some(path_text.clone()),
                None,
                Some(vec![file_manager.to_string_lossy().into_owned(), path_text]),
            )
        })
        .collect()
}

fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn nickel_file_executable() -> PathBuf {
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("nickel-ui"));
    #[cfg(target_os = "windows")]
    return executable.with_file_name("nickel-file.exe");
    #[cfg(not(target_os = "windows"))]
    executable.with_file_name("nickel-file")
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::nickel_file_executable;

    #[test]
    fn places_launch_the_sibling_file_manager() {
        let executable = nickel_file_executable();
        #[cfg(target_os = "windows")]
        assert_eq!(
            executable.file_name().and_then(|name| name.to_str()),
            Some("nickel-file.exe")
        );
        #[cfg(not(target_os = "windows"))]
        assert_eq!(
            executable.file_name().and_then(|name| name.to_str()),
            Some("nickel-file")
        );
    }
}
