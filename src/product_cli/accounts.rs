//! Explicit native login over the same owner-checked routes as the UI.
use super::*;
use crate::provider_account::ProviderAccountId;

#[derive(Args)]
pub struct AccountArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    #[command(subcommand)]
    command: AccountCommand,
}

#[derive(Subcommand)]
enum AccountCommand {
    /// List this operator's subscriptions and registry revision.
    List,
    /// Apply one metadata change from stdin; use dataset/revision from `account list`.
    /// Reuse operation, revision, dataset and identical input after uncertainty.
    Update {
        #[arg(long)]
        operation: String,
        #[arg(long)]
        dataset: String,
        #[arg(long)]
        expected_revision: i64,
    },
    /// Inspect local login readiness. Does not contact a Provider or create a vault.
    LoginOptions {
        #[arg(value_parser = parse_account)]
        account: ProviderAccountId,
        #[arg(long)]
        machine: String,
    },
    /// Start or resume an explicit native login on one Machine. Prints its request ID.
    /// No automatic transport retry, account rotation or fallback.
    Login {
        /// Stable ID for this login attempt. Reuse with identical input after uncertainty.
        #[arg(long)]
        operation: String,
        #[arg(value_parser = parse_account)]
        account: ProviderAccountId,
        #[arg(long)]
        machine: String,
        #[arg(long)]
        provider_version: String,
        #[arg(long)]
        generation_digest: String,
        #[arg(long)]
        method: String,
    },
    /// Read one login's events; pending credential promotion may be reconciled.
    /// Output can contain a private sign-in link. Does not loop or poll automatically.
    Events { provider: String, request: String },
    /// Submit the native login code from stdin (never a command-line argument).
    SubmitCode { provider: String, request: String },
    /// Persist cancellation for a login operation, including an uncertain dispatch.
    /// Does not revoke an already committed subscription or prove process exit.
    Cancel { operation: String },
    /// Observe a durable login reservation without starting a native process.
    Receipt { operation: String },
}

pub(super) fn parse_account(value: &str) -> Result<ProviderAccountId, String> {
    ProviderAccountId::try_from(value.to_owned()).map_err(|error| error.to_string())
}

fn login_path(account: &ProviderAccountId) -> Result<String> {
    ensure!(
        !account.is_default(),
        "native subscription login requires a named account ID"
    );
    Ok(format!("/api/provider-accounts/{}/login", account.as_str()))
}

fn request_path(provider: &str, request: &str) -> Result<String> {
    ensure!(
        [provider, request]
            .iter()
            .all(|value| crate::managed_calls::valid_id(value) && !matches!(*value, "." | "..")),
        "invalid Provider or login request ID"
    );
    Ok(format!("/api/providers/{provider}/auth/{request}"))
}

#[cfg(test)]
mod tests;

fn read_code(reader: impl std::io::Read) -> Result<String> {
    use std::io::Read as _;
    let mut value = String::new();
    reader
        .take(4097)
        .read_to_string(&mut value)
        .context("reading native login code from stdin")?;
    ensure!(value.len() <= 4096, "native login code exceeds 4096 bytes");
    crate::provider_account::login_code(&value)
        .map(str::to_owned)
        .map_err(|error| anyhow!(error))
}

fn read_change(reader: impl std::io::Read) -> Result<crate::store::provider_accounts::Change> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    reader
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .context("reading account metadata change")?;
    ensure!(
        bytes.len() <= 16 * 1024,
        "account metadata change exceeds 16 KiB"
    );
    serde_json::from_slice(&bytes).map_err(|_| {
        anyhow!(
            "invalid account metadata change; expected create, rename, set_enabled or set_default"
        )
    })
}

pub async fn run(args: AccountArgs) -> Result<()> {
    let client = Client::new(&args.connect)?;
    let response = execute(&client, args.command, std::io::stdin()).await?;
    println!("{response}");
    Ok(())
}

async fn execute(
    client: &Client,
    command: AccountCommand,
    input: impl std::io::Read,
) -> Result<String> {
    match command {
        AccountCommand::List => {
            client
                .expect(Method::GET, "/api/provider-accounts", None)
                .await
        }
        AccountCommand::Update {
            operation,
            dataset,
            expected_revision,
        } => {
            ensure!(
                crate::managed_calls::valid_id(&operation),
                "invalid account operation ID"
            );
            ensure!(
                !dataset.is_empty() && expected_revision >= 0,
                "dataset and nonnegative revision are required"
            );
            let change = read_change(input)?;
            client.expect(Method::PUT, "/api/provider-accounts", Some(&json!({"operation":operation,"dataset":dataset,"expected_revision":expected_revision,"change":change}))).await
        }
        AccountCommand::LoginOptions { account, machine } => {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("machine_id", &machine)
                .finish();
            client
                .expect(
                    Method::GET,
                    &format!("{}?{query}", login_path(&account)?),
                    None,
                )
                .await
        }
        AccountCommand::Login {
            operation,
            account,
            machine,
            provider_version,
            generation_digest,
            method,
        } => {
            ensure!(
                crate::managed_calls::valid_id(&operation),
                "invalid login operation ID"
            );
            let body = json!({"operation":operation,"machine_id":machine,"provider_version":provider_version,"generation_digest":generation_digest,"method":method});
            client
                .expect(Method::POST, &login_path(&account)?, Some(&body))
                .await
        }
        AccountCommand::Events { provider, request } => {
            client
                .expect(Method::GET, &request_path(&provider, &request)?, None)
                .await
        }
        AccountCommand::SubmitCode { provider, request } => {
            let path = request_path(&provider, &request)?;
            let code = read_code(input)?;
            client
                .expect(Method::POST, &path, Some(&json!({"code":code})))
                .await
        }
        AccountCommand::Cancel { ref operation } | AccountCommand::Receipt { ref operation } => {
            ensure!(
                crate::managed_calls::valid_id(operation)
                    && !matches!(operation.as_str(), "." | ".."),
                "invalid login operation ID"
            );
            client
                .expect(
                    if matches!(command, AccountCommand::Cancel { .. }) {
                        Method::DELETE
                    } else {
                        Method::GET
                    },
                    &format!("/api/provider-accounts/login-operations/{operation}"),
                    None,
                )
                .await
        }
    }
}
