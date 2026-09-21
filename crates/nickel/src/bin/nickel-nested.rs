fn main() -> Result<(), String> {
    nickel_shell::session::run_nested().map_err(|error| error.to_string())
}
