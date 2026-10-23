use std::collections::HashMap;
use std::io::Write;

use crate::account::account::Account;
use crate::client::client_id::ClientId;
use crate::processing_error::ProcessingError;

pub const HEADER: &str = "client,available,held,total,locked";

/// Writes the accounts CSV. Row order is unspecified; this sorts by client for
/// reproducible output.
pub fn write_accounts<W: Write>(
    out: &mut W,
    accounts: &HashMap<ClientId, Account>,
) -> Result<(), ProcessingError> {
    writeln!(out, "{HEADER}")?;
    let mut clients: Vec<ClientId> = accounts.keys().copied().collect();
    clients.sort_unstable();
    for client in clients {
        let account = &accounts[&client];
        writeln!(
            out,
            "{},{},{},{},{}",
            client,
            account.available,
            account.held,
            account.total(),
            account.locked
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(value: &str) -> crate::amount::Amount {
        crate::amount::Amount::parse(value).expect("valid amount")
    }

    #[test]
    fn writes_the_header_for_an_empty_map() {
        let mut out = Vec::new();
        write_accounts(&mut out, &HashMap::new()).expect("write succeeds");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "client,available,held,total,locked\n"
        );
    }

    #[test]
    fn writes_accounts_sorted_by_client() {
        let mut accounts = HashMap::new();
        accounts.insert(2, Account::default());
        let mut first = Account::default();
        first.credit(amount("1.5")).unwrap();
        accounts.insert(1, first);

        let mut out = Vec::new();
        write_accounts(&mut out, &accounts).expect("write succeeds");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("1,1.5,0.0,1.5,false\n"));
        assert!(text.contains("2,0.0,0.0,0.0,false\n"));
    }
}
