use std::{env, path::PathBuf, process::ExitCode};

use nickel_markdown_ui::ViewerApplication;

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .iter()
        .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        println!(
            "nickel-markdown-ui PATH\n\nOpen one local .md or .markdown file in a read-only Nickel viewer."
        );
        return ExitCode::SUCCESS;
    }
    if arguments.len() != 1 {
        eprintln!("usage: nickel-markdown-ui PATH");
        return ExitCode::from(2);
    }
    match nickel_ui::run(ViewerApplication::open(PathBuf::from(&arguments[0]))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Nickel Markdown failed: {error}");
            ExitCode::FAILURE
        }
    }
}
