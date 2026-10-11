//! Command line entry point: hand argv to the library and print the result.

use std::process::ExitCode;

use rust_loc::cli;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("rust-loc: cannot read cwd: {}", error);
            return ExitCode::from(2);
        }
    };
    match cli::run(&arguments, &cwd) {
        Ok(output) => {
            print!("{}", output);
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("rust-loc: {}", message);
            ExitCode::from(2)
        }
    }
}
