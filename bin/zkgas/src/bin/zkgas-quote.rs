use clap::Parser;
use zkgas::{
    identity::executable_identity,
    quote::{QuoteArgs, run_quote},
};

fn main() {
    let args = match QuoteArgs::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 1 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };
    let result = executable_identity().and_then(|executable| run_quote(&args, &executable));
    let code = match result {
        Ok(run) => run.exit_code(),
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
