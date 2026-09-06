#[global_allocator]
static GLOBAL_ALLOCATOR: nickel_shell::allocation_counter::CountingSystemAllocator =
    nickel_shell::allocation_counter::CountingSystemAllocator;

fn main() -> Result<(), String> {
    nickel_shell::run()
}
