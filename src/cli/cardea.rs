//! Offline discovery artifacts for the app-owned generic authorization adapter.
use anyhow::{Result, ensure};
use cardea_core::authorization::{Catalog, CatalogProposal};
use clap::Subcommand;
use serde_json::json;

#[derive(Subcommand)]
pub(super) enum Command {
    /// Print a catalog proposal; use cardea change propose for human review.
    Catalog {
        #[arg(long)]
        application: String,
        #[arg(long)]
        origin: String,
        #[arg(long)]
        change_id: String,
        #[arg(long)]
        expected_revision: u64,
    },
}
pub(crate) fn proposal(
    application: &str,
    origin: &str,
    id: &str,
    expected: u64,
) -> Result<CatalogProposal> {
    ensure!(
        cardea_core::authorization::opaque_id(id),
        "Save an ID from cardea agent id"
    );
    let revision = expected
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("revision overflow"))?;
    let value = json!({"schema":"dravengarden.cardea.action-catalog/v1","application_id":application,"revision":revision,"adapter_origin":origin,
        "permission_model":"Cowboy operator membership, signed releases and Machine installation CAS",
        "actions":[{"id":"plugin.install","title":"Install Cowboy Plugin","summary":"Review one exact signed release and its current Machine target; reuse the durable installer without terminating active sessions","effect":"write","approval":{"reviewers":1,"factor":"transaction_factor"},"permissions":["cowboy.machine.plugin.install"],"inputs":[
        {"name":"machine","kind":"string","required":true,"description":"Exact registered Machine identity"},
        {"name":"plugin","kind":"string","required":true,"description":"Exact trusted Plugin identity"},
        {"name":"version","kind":"string","required":true,"description":"Exact signed release version"},
        {"name":"digest","kind":"string","required":true,"description":"Exact signed artifact digest"},
        {"name":"target","kind":"object","required":false,"description":"Server-populated installation precondition; callers omit this field"},
        {"name":"envelope_digest","kind":"string","required":false,"description":"Server-populated trusted release envelope hash; callers omit this field"}]}]});
    let catalog: Catalog = serde_json::from_value(value)?;
    ensure!(
        catalog.validate(),
        "Invalid application, origin or catalog revision"
    );
    Ok(CatalogProposal {
        change_id: id.into(),
        expected_revision: expected,
        catalog,
    })
}
pub(super) fn run(command: Command) -> Result<()> {
    match command {
        Command::Catalog {
            application,
            origin,
            change_id,
            expected_revision,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&proposal(
                &application,
                &origin,
                &change_id,
                expected_revision
            )?)?
        ),
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_uses_real_protocol_validation_and_closed_inputs() {
        let p = proposal(
            "cowboy-production",
            "https://cowboy.stormbird.xyz",
            &"A".repeat(43),
            0,
        )
        .unwrap();
        assert!(p.catalog.validate());
        assert!(p.catalog.action("plugin.install").unwrap().validate_input(&json!({"machine":"hawk","plugin":"codex","version":"1.0.0","digest":format!("sha256:{}","0".repeat(64))})));
        for origin in [
            "http://cowboy.stormbird.xyz",
            "https://evil.example/path",
            "https://127.0.0.1",
            "https://cowboy.stormbird.xyz:443",
        ] {
            assert!(proposal("cowboy-production", origin, &"A".repeat(43), 0).is_err());
        }
        assert!(
            proposal(
                "cowboy-production",
                "https://cowboy.stormbird.xyz",
                &"A".repeat(43),
                u64::MAX
            )
            .is_err()
        );
    }
}
