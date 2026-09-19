use clap::{Parser, Subcommand};

use grpcping::call::unary_call;
use grpcping::reflection::{connect, list_services, resolve_service};

#[derive(Parser)]
#[command(
    name = "grpcping",
    about = "A grpcurl-equivalent: lists services and makes dynamic unary gRPC calls via server reflection"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List every service the server exposes via reflection.
    List { addr: String },
    /// Describe a service's methods and their input/output message types.
    Describe {
        addr: String,
        /// Fully-qualified service name, e.g. `greeter.Greeter`.
        service: String,
    },
    /// Make a unary call.
    Call {
        addr: String,
        /// `package.Service/Method`, e.g. `greeter.Greeter/SayHello`.
        method: String,
        /// JSON request payload.
        #[arg(short = 'd', long = "data", default_value = "{}")]
        data: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::List { addr } => {
            let channel = connect(&addr).await.map_err(|e| anyhow::anyhow!(e))?;
            let services = list_services(channel)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            for name in services {
                println!("{name}");
            }
        }
        Command::Describe { addr, service } => {
            let channel = connect(&addr).await.map_err(|e| anyhow::anyhow!(e))?;
            let (_pool, svc) = resolve_service(channel, &service)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            for method in svc.methods() {
                println!(
                    "{} ({}) returns ({})",
                    method.name(),
                    method.input().full_name(),
                    method.output().full_name()
                );
            }
        }
        Command::Call { addr, method, data } => {
            let (service_name, method_name) = method.rsplit_once('/').ok_or_else(|| {
                anyhow::anyhow!("expected package.Service/Method, got '{method}'")
            })?;

            let channel = connect(&addr).await.map_err(|e| anyhow::anyhow!(e))?;
            let (_pool, svc) = resolve_service(channel.clone(), service_name)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            let method_desc = svc
                .methods()
                .find(|m| m.name() == method_name)
                .ok_or_else(|| {
                    anyhow::anyhow!("method '{method_name}' not found on {service_name}")
                })?;

            let response = unary_call(channel, &method_desc, &data)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
    }

    Ok(())
}
