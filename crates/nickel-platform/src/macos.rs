pub fn open_external_url(url: &str) -> Result<(), String> {
    external_url_command(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start the system URL handler: {error}"))
}

fn external_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(url);
    command
}

#[cfg(test)]
mod tests {
    use super::external_url_command;

    #[test]
    fn external_url_command_preserves_the_argument() {
        let command = external_url_command("https://example.test/a path?x=1&y=2");
        assert_eq!(command.get_program(), "open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["https://example.test/a path?x=1&y=2"]
        );
    }
}
