#[global_allocator]
static GLOBAL_ALLOCATOR: nickel_shell::allocation_counter::CountingSystemAllocator =
    nickel_shell::allocation_counter::CountingSystemAllocator;

fn main() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    return nickel_shell::session::run().map_err(|error| error.to_string());

    #[cfg(not(target_os = "linux"))]
    nickel_shell::run()
}
