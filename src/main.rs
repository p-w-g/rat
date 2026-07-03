mod cli;
mod commands;
mod config;
mod exec;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let Some(command) = args.first() else {
        println!("Usage: rat <command> [arguments]");
        return;
    };
    let command = command.to_lowercase();
    let instance = cli::parse_instance(&args[1..]);

    match command.as_str() {
        "help" | "h" | "-h" | "--h" | "-help" | "--help" => commands::help::show_help(),
        "fep" => {
            if !commands::fep::run_parallel(&instance) {
                std::process::exit(1);
            }
        }
        "cfg" => config::evaluate(&instance),
        _ => println!("Unknown command: {command} - refer to help (rat help)"),
    }
}
