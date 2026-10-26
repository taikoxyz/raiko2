use clap::Parser;
use zkgas::{
    compare::{CompareArgs, run_compare},
    identity::executable_identity,
};

fn main() {
    let args = match CompareArgs::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 1 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };
    let result = executable_identity().and_then(|executable| run_compare(&args, &executable));
    let code = match result {
        Ok(run) => {
            println!("{}", run.manifest.run_id);
            run.exit_code()
        }
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
