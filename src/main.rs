use clap::Parser;
use clap::builder::Styles;
use clap::builder::styling::AnsiColor;
use phpantom_lsp::Backend;
use phpantom_lsp::config;
use tower_lsp::{LspService, Server};

const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default().bold())
    .usage(AnsiColor::Yellow.on_default().bold())
    .literal(AnsiColor::Green.on_default().bold())
    .placeholder(AnsiColor::Green.on_default());

#[derive(Parser)]
#[command(name = "phpantom_lsp", styles = STYLES)]
#[command(
    version,
    about = "A fast and lightweight PHP Language Server Protocol implementation"
)]
struct Cli {
    /// Create a default .phpantom.toml configuration file in the current directory and exit.
    #[arg(long)]
    init: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if cli.init {
        let cwd = std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("Error: cannot determine current directory: {}", e);
            std::process::exit(1);
        });

        match config::create_default_config(&cwd) {
            Ok(true) => {
                println!("Created {} in {}", config::CONFIG_FILE_NAME, cwd.display());
            }
            Ok(false) => {
                println!(
                    "{} already exists in {}",
                    config::CONFIG_FILE_NAME,
                    cwd.display()
                );
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }

        return;
    }

    env_logger::init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::build(Backend::new)
        .custom_method("textDocument/inlayHint", Backend::inlay_hint_request)
        .finish();

    Server::new(stdin, stdout, socket).serve(service).await;
}
