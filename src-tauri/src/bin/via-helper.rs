#[allow(dead_code, unused_imports)]
#[path = "../helper/mod.rs"]
mod helper;

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [command] if command == "serve" => {
            if let Err(error) = helper::run_server().await {
                eprintln!("via-helper failed: {error}");
                std::process::exit(1);
            }
        }
        [command] if command == "--version" || command == "-V" => {
            println!(
                "via-helper {} protocol {}",
                env!("CARGO_PKG_VERSION"),
                helper::HELPER_PROTOCOL_VERSION
            );
        }
        [command] if command == "--help" || command == "-h" => print_help(),
        [] => print_help(),
        _ => {
            eprintln!("via-helper accepts only the `serve` operation");
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "via-helper {}\n\nUSAGE:\n    via-helper serve\n\nThe helper is started by the platform installer and accepts only authenticated local IPC.",
        env!("CARGO_PKG_VERSION")
    );
}
