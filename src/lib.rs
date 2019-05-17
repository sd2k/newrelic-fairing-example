use newrelic;

use rocket::{
    fairing::{Fairing, Info, Kind},
    request::{self, FromRequest},
    Data, Outcome, Request, Response,
};

/// A fairing used to instrument requests with New Relic.
pub struct NewRelic(newrelic::App);

impl NewRelic {
    pub fn new(app_name: &str, license_key: &str) -> Self {
        NewRelic(newrelic::App::new(app_name, license_key).unwrap())
    }
}

impl Fairing for NewRelic {
    fn info(&self) -> Info {
        Info {
            name: "New Relic instrumentation",
            kind: Kind::Request | Kind::Response,
        }
    }

    fn on_request(&self, request: &mut Request, _: &Data) {
        request.local_cache(|| Transaction::new(&self.0, request));
    }

    /// End the New Relic transaction, if it has been used in a request guard.
    ///
    /// Also adds an error code to the transaction if the response did
    /// not succeed.
    fn on_response(&self, request: &Request, response: &mut Response) {
        match request.local_cache(|| Transaction::None) {
            Transaction::Traced(transaction) => {
                // Record any errors
                let status = response.status();
                if !status.class().is_success() {
                    transaction.notice_error(100, &status.to_string(), "").ok();
                }
                // End the transaction explicitly here.
                // Otherwise it ends after the response has finished being
                // sent to the client, when it's dropped.

                // **This doesn't work** because `Transaction::end` requires `&mut self`
                transaction.end();
            }
            Transaction::NotTraced(transaction) => {
                transaction.ignore();
            }
            _ => {}
        }
    }
}

pub enum Transaction {
    /// A running and traced New Relic transaction.
    /// Will be sent to New Relic.
    Traced(newrelic::Transaction),

    /// A running, but not traced, New Relic transaction.
    /// Will be ignored rather than ended.
    NotTraced(newrelic::Transaction),

    /// A dummy transaction; used if the New Relic SDK
    /// returns an error.
    None,
}

impl Transaction {
    // Create a new transaction for a request.
    //
    // The New Relic transaction will have the URL and transaction name
    // attributes set.
    fn new(app: &newrelic::App, request: &Request) -> Self {
        // Use the route handler as the transaction name.
        // This should always be used inside a request guard so that
        // request.route() is not None.
        let transaction_name: String = request
            .route()
            .map(|r| {
                format!(
                    "{}/{}",
                    r.base.to_string().trim_start_matches('/'),
                    r.name.unwrap_or("unknown_handler")
                )
            })
            .unwrap_or_else(|| "unknown_handler".to_string());

        // Start a new transaction. Note that this will be ignored
        // unless it's used in a request guard.
        app.web_transaction(&transaction_name)
            .map(|t| Transaction::NotTraced(t))
            .unwrap_or(Transaction::None)
    }
}

impl<'a, 'r> FromRequest<'a, 'r> for &'a Transaction {
    type Error = ();

    // Switch the transaction from a NonTraced to a Traced transaction.
    fn from_request(request: &'a Request<'r>) -> request::Outcome<Self, Self::Error> {
        let transaction = request.local_cache(|| Transaction::None);
        if let Transaction::NotTraced(t) = transaction {
            // This doesn't work because we don't own, and can't mutate,
            // the request-local cache.
            *transaction = Transaction::Traced(*t);
        }
        Outcome::Success(transaction)
    }
}
