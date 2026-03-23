use spin_cli::subprocess::ExitStatusError;

#[tokio::main]
async fn main() {
    if let Err(err) = spin_cli::run().await {
        let code = match err.downcast_ref::<ExitStatusError>() {
            // If we encounter an `ExitStatusError` it means a subprocess has already
            // exited unsuccessfully and thus already printed error messages. No need
            // to print anything additional.
            Some(e) => e.code(),
            // Otherwise we print the error chain.
            None => {
                terminal::error!("{err}");
                print_error_chain(err);
                1
            }
        };

        std::process::exit(code)
    }
}

fn print_error_chain(err: anyhow::Error) {
    if let Some(cause) = err.source() {
        let is_multiple = cause.source().is_some();
        eprintln!("\nCaused by:");
        for (i, err) in err.chain().skip(1).enumerate() {
            if is_multiple {
                eprintln!("{i:>4}: {err}")
            } else {
                eprintln!("      {err}")
            }
        }
    }
}
