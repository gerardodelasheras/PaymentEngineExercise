//! Final account-state report: the accounts CSV writer.

use std::collections::HashMap;
use std::io::Write;

use super::Account;
use crate::client::ClientId;
use crate::processing_error::ProcessingError;

/// Write the accounts CSV header and one row per initialized account.
pub fn write_accounts<W: Write>(
    writer: &mut W,
    accounts: &HashMap<ClientId, Account>,
) -> Result<(), ProcessingError> {
    writeln!(writer, "client,available,held,total,locked")?;
    for (id, account) in accounts {
        writeln!(
            writer,
            "{},{},{},{},{}",
            id,
            account.available,
            account.held,
            account.total(),
            account.locked
        )?;
    }
    Ok(())
}
