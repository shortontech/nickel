use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let get = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|index| args.get(index + 1))
            .map(PathBuf::from)
    };
    let (Some(manifest), Some(archives), Some(output), Some(target), Some(license)) = (
        get("--manifest"),
        get("--archives"),
        get("--output"),
        args.iter()
            .position(|arg| arg == "--target")
            .and_then(|index| args.get(index + 1))
            .cloned(),
        get("--license"),
    ) else {
        eprintln!(
            "usage: nickel-codex-bundle --manifest FILE --archives DIR --output DIR --target TRIPLE --license FILE"
        );
        return ExitCode::from(2);
    };
    match nickel_codex::stage_bundle(&manifest, &archives, &output, &target, &license) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
