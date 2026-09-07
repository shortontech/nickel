#[global_allocator]
static GLOBAL_ALLOCATOR: nickel_shell::allocation_counter::CountingSystemAllocator =
    nickel_shell::allocation_counter::CountingSystemAllocator;

fn main() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if matches!(
        std::env::args_os().nth(1).as_deref(),
        Some(argument)
            if argument == std::ffi::OsStr::new("--backend")
                || argument == std::ffi::OsStr::new("--available-backends")
                || argument == std::ffi::OsStr::new("--test-control")
                || argument == std::ffi::OsStr::new("--shell-process")
                || argument == std::ffi::OsStr::new("--ui-renderer")
    ) {
        return nickel_shell::session::run().map_err(|error| error.to_string());
    }

    nickel_shell::run()
}
